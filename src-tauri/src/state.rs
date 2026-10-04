use crate::alerts::AlertThresholds;
use crate::client::{KeySwitchError, Sub2Client};
use crate::history::History;
use crate::models::{
    ApiKeyBinding, AppSettings, AppSettingsPublic, AppStateView, BindableGroup, SiteConfig,
    SitePublic, SiteRole, SiteSnapshot, SiteUpsert,
};
use crate::store::Store;
use chrono::Utc;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::RwLock;
use uuid::Uuid;

pub struct AppState {
    pub store: Store,
    pub client: Sub2Client,
    pub settings: RwLock<AppSettings>,
    pub snapshots: RwLock<HashMap<String, SiteSnapshot>>,
    pub last_refresh_at: RwLock<Option<String>>,
    pub refreshing: RwLock<bool>,
    switching_keys: Mutex<HashSet<String>>,
    history: Mutex<History>,
}

impl AppState {
    pub fn new() -> Result<Self, String> {
        let store = Store::new().map_err(|e| e.to_string())?;
        let settings = store.load_settings().map_err(|e| e.to_string())?;
        Ok(Self {
            store,
            client: Sub2Client::new(),
            settings: RwLock::new(settings),
            snapshots: RwLock::new(HashMap::new()),
            last_refresh_at: RwLock::new(None),
            refreshing: RwLock::new(false),
            switching_keys: Mutex::new(HashSet::new()),
            history: Mutex::new(History::load()),
        })
    }

    pub async fn public_settings(&self) -> AppSettingsPublic {
        let settings = self.settings.read().await;
        AppSettingsPublic {
            refresh_interval_secs: settings.refresh_interval_secs,
            low_balance_threshold: settings.low_balance_threshold,
            warn_balance_usd: settings.warn_balance_usd,
            critical_balance_usd: settings
                .critical_balance_usd
                .max(settings.low_balance_threshold),
            warn_health_pct: settings.warn_health_pct,
            critical_health_pct: settings.critical_health_pct,
            warn_available_count: settings.warn_available_count,
            critical_available_count: settings.critical_available_count,
            launch_at_login: settings.launch_at_login,
            sites: settings.sites.iter().map(Store::to_public).collect(),
        }
    }

    pub async fn view(&self) -> AppStateView {
        let settings = self.public_settings().await;
        let map = self.snapshots.read().await;
        let mut snapshots: Vec<SiteSnapshot> = settings
            .sites
            .iter()
            .filter_map(|s| map.get(&s.id).cloned())
            .collect();
        // include any orphan snapshots
        for (id, snap) in map.iter() {
            if !snapshots.iter().any(|s| s.site.id == *id) {
                snapshots.push(snap.clone());
            }
        }
        snapshots.sort_by(|a, b| a.site.name.cmp(&b.site.name));
        let now_ms = Utc::now().timestamp_millis();
        {
            let history = self.history.lock().unwrap_or_else(|p| p.into_inner());
            for snap in &mut snapshots {
                snap.delta = history.delta_for(snap, now_ms);
            }
        }
        AppStateView {
            settings,
            snapshots,
            last_refresh_at: self.last_refresh_at.read().await.clone(),
            refreshing: *self.refreshing.read().await,
        }
    }

    pub async fn emit_state(&self, app: &AppHandle) {
        let view = self.view().await;
        let _ = app.emit("state-updated", view);
    }

    pub async fn upsert_site(&self, input: SiteUpsert) -> Result<SitePublic, String> {
        let base_url = Store::normalize_base_url(&input.base_url);
        let id = input
            .id
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        let mut settings = self.settings.write().await;
        let existing = settings.sites.iter().position(|s| s.id == id);

        let mut site = SiteConfig {
            id: id.clone(),
            name: input.name.trim().to_string(),
            base_url,
            role: input.role.clone(),
            api_key_label: input.api_key_label.clone().filter(|s| !s.trim().is_empty()),
            email: input.email.clone().filter(|s| !s.trim().is_empty()),
            enabled: input.enabled.unwrap_or(true),
        };
        Store::validate_site(&site)?;

        match site.role {
            SiteRole::User => {
                if let Some(key) = input.api_key.as_ref().filter(|s| !s.is_empty()) {
                    Store::set_secret(&id, "api_key", key).map_err(|e| e.to_string())?;
                } else if existing.is_none() {
                    return Err("用户模式需要提供 API Key".into());
                } else if !Store::has_secret(&id, "api_key") {
                    return Err("用户模式需要提供 API Key".into());
                }
            }
            SiteRole::Admin => {
                if let Some(pw) = input.password.as_ref().filter(|s| !s.is_empty()) {
                    Store::set_secret(&id, "password", pw).map_err(|e| e.to_string())?;
                    // clear old tokens so next refresh re-logins
                    Store::delete_secret(&id, "access_token");
                    Store::delete_secret(&id, "refresh_token");
                } else if existing.is_none() && !Store::has_secret(&id, "password") {
                    return Err("管理员模式需要提供密码".into());
                }
            }
        }

        if let Some(idx) = existing {
            // preserve label/email if not provided? already set from input
            let old = &settings.sites[idx];
            if site.api_key_label.is_none() {
                site.api_key_label = old.api_key_label.clone();
            }
            settings.sites[idx] = site.clone();
        } else {
            settings.sites.push(site.clone());
        }

        self.store
            .save_settings(&settings)
            .map_err(|e| e.to_string())?;
        Ok(Store::to_public(&site))
    }

    pub async fn delete_site(&self, site_id: &str) -> Result<(), String> {
        let mut settings = self.settings.write().await;
        let before = settings.sites.len();
        settings.sites.retain(|s| s.id != site_id);
        if settings.sites.len() == before {
            return Err("站点不存在".into());
        }
        self.store
            .save_settings(&settings)
            .map_err(|e| e.to_string())?;
        Store::delete_all_secrets(site_id);
        self.snapshots.write().await.remove(site_id);
        {
            let mut history = self.history.lock().unwrap_or_else(|p| p.into_inner());
            history.remove(site_id);
            history.save();
        }
        Ok(())
    }

    pub async fn update_settings(
        &self,
        refresh_interval_secs: Option<u64>,
        low_balance_threshold: Option<f64>,
        warn_balance_usd: Option<f64>,
        critical_balance_usd: Option<f64>,
        warn_health_pct: Option<f64>,
        critical_health_pct: Option<f64>,
        warn_available_count: Option<i64>,
        critical_available_count: Option<i64>,
        launch_at_login: Option<bool>,
    ) -> Result<(), String> {
        let mut settings = self.settings.write().await;
        if let Some(v) = refresh_interval_secs {
            settings.refresh_interval_secs = v.clamp(15, 3600);
        }
        if let Some(v) = warn_balance_usd {
            settings.warn_balance_usd = v.max(0.0);
        }
        if let Some(v) = critical_balance_usd.or(low_balance_threshold) {
            settings.critical_balance_usd = v.max(0.0);
            settings.low_balance_threshold = settings.critical_balance_usd;
        }
        if settings.warn_balance_usd < settings.critical_balance_usd {
            settings.warn_balance_usd = settings.critical_balance_usd;
        }
        if let Some(v) = warn_health_pct {
            settings.warn_health_pct = v.clamp(1.0, 100.0);
        }
        if let Some(v) = critical_health_pct {
            settings.critical_health_pct = v.clamp(0.0, 100.0);
        }
        if settings.warn_health_pct < settings.critical_health_pct {
            settings.warn_health_pct = settings.critical_health_pct;
        }
        if let Some(v) = warn_available_count {
            settings.warn_available_count = v.max(0);
        }
        if let Some(v) = critical_available_count {
            settings.critical_available_count = v.max(0);
        }
        if settings.warn_available_count < settings.critical_available_count {
            settings.warn_available_count = settings.critical_available_count;
        }
        if let Some(enabled) = launch_at_login {
            settings.launch_at_login = enabled;
        }
        self.store
            .save_settings(&settings)
            .map_err(|e| e.to_string())
    }

    pub async fn save_panel_position(&self, x: f64, y: f64) {
        let mut settings = self.settings.write().await;
        if settings.panel_x == Some(x) && settings.panel_y == Some(y) {
            return;
        }
        settings.panel_x = Some(x);
        settings.panel_y = Some(y);
        let _ = self.store.save_settings(&settings);
    }

    pub async fn clear_panel_position(&self) {
        let mut settings = self.settings.write().await;
        settings.panel_x = None;
        settings.panel_y = None;
        let _ = self.store.save_settings(&settings);
    }

    pub async fn refresh_all(&self, app: &AppHandle) {
        {
            let mut r = self.refreshing.write().await;
            if *r {
                return;
            }
            *r = true;
        }
        let _ = app.emit("refreshing", true);

        let sites: Vec<SiteConfig> = {
            let settings = self.settings.read().await;
            settings
                .sites
                .iter()
                .filter(|s| s.enabled)
                .cloned()
                .collect()
        };

        let client = self.client.clone();
        let mut handles = Vec::new();
        for site in sites {
            let client = client.clone();
            handles.push(tokio::spawn(
                async move { refresh_one(&client, site).await },
            ));
        }

        let mut new_map = HashMap::new();
        for h in handles {
            if let Ok(snap) = h.await {
                new_map.insert(snap.site.id.clone(), snap);
            }
        }

        {
            let mut map = self.snapshots.write().await;
            // remove disabled sites
            let enabled_ids: std::collections::HashSet<_> = new_map.keys().cloned().collect();
            map.retain(|k, _| enabled_ids.contains(k));
            for (k, v) in new_map {
                map.insert(k, v);
            }
        }

        let refreshed_at = Utc::now();
        *self.last_refresh_at.write().await = Some(refreshed_at.to_rfc3339());
        *self.refreshing.write().await = false;

        let thresholds = {
            let settings = self.settings.read().await;
            AlertThresholds {
                warn_balance: settings.warn_balance_usd,
                critical_balance: settings.critical_balance_usd,
                warn_available: settings.warn_available_count,
                critical_available: settings.critical_available_count,
            }
        };
        let keep_ids = {
            let settings = self.settings.read().await;
            settings.sites.iter().map(|site| site.id.clone()).collect()
        };
        let notes = {
            let map = self.snapshots.read().await;
            let mut history = self.history.lock().unwrap_or_else(|p| p.into_inner());
            let mut notes = Vec::new();
            let now_ms = refreshed_at.timestamp_millis();
            for snap in map.values() {
                history.record_sample(snap, now_ms);
                if let Some(body) = history.take_alert(snap, thresholds) {
                    notes.push(body);
                }
            }
            history.retain(&keep_ids);
            history.save();
            notes
        };

        let _ = app.emit("refreshing", false);
        self.emit_state(app).await;
        update_tray_tooltip(app, self).await;
        let view = self.view().await;
        crate::tray::apply_tray_menu(app, &view);
        for body in notes {
            notify(app, &body);
        }
    }

    pub async fn switch_key_group(
        &self,
        app: &AppHandle,
        site_id: String,
        key_id: i64,
        group_id: i64,
    ) {
        let guard_id = format!("{site_id}|{key_id}");
        if !self.try_begin_switch(&guard_id) {
            return;
        }
        let changed = self.apply_key_switch(&site_id, key_id, group_id).await;
        self.end_switch(&guard_id);
        if changed {
            self.emit_state(app).await;
            let view = self.view().await;
            crate::tray::apply_tray_menu(app, &view);
        }
    }

    fn try_begin_switch(&self, id: &str) -> bool {
        let mut set = self
            .switching_keys
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if !set.insert(id.to_string()) {
            return false;
        }
        true
    }

    fn end_switch(&self, id: &str) {
        self.switching_keys
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id);
    }

    async fn apply_key_switch(&self, site_id: &str, key_id: i64, group_id: i64) -> bool {
        let (base_url, email) = {
            let settings = self.settings.read().await;
            let Some(site) = settings.sites.iter().find(|s| s.id == site_id) else {
                return false;
            };
            if site.role != SiteRole::Admin {
                return false;
            }
            (site.base_url.clone(), site.email.clone())
        };
        {
            let map = self.snapshots.read().await;
            let Some(admin) = map.get(site_id).and_then(|s| s.admin.as_ref()) else {
                return false;
            };
            let groups_missing = admin.bindable_groups.is_empty() && admin.key_list_error.is_some();
            if !admin.key_switch_supported || groups_missing {
                return false;
            }
            let Some(key) = admin.api_keys.iter().find(|k| k.id == key_id) else {
                return false;
            };
            if key.group_id.unwrap_or(0) == group_id {
                return false;
            }
        }

        match self
            .client
            .switch_api_key_group(site_id, &base_url, email.as_deref(), key_id, group_id)
            .await
        {
            Ok(()) => {
                log::info!("switched api key {key_id} to group {group_id}");
                match self
                    .client
                    .load_key_state(site_id, &base_url, email.as_deref())
                    .await
                {
                    Ok((cat, supported)) => {
                        self.write_key_catalog(
                            site_id,
                            cat.keys,
                            cat.groups,
                            cat.truncated,
                            cat.list_error,
                            supported,
                        )
                        .await;
                    }
                    Err(e) => {
                        log::warn!("key list refresh after switch failed: {e}");
                        self.apply_local_group(site_id, key_id, group_id).await;
                    }
                }
                true
            }
            Err(KeySwitchError::Unsupported) => {
                self.client.remember_switch_support(&base_url, false);
                self.set_key_error(site_id, key_id, "当前站点不能切换分组".into())
                    .await;
                self.mark_switch_unsupported(site_id).await;
                true
            }
            Err(KeySwitchError::Unauthorized) => {
                self.set_key_error(site_id, key_id, "登录已过期，请在设置中重新登录".into())
                    .await;
                true
            }
            Err(KeySwitchError::Failed(msg)) => {
                self.set_key_error(site_id, key_id, msg).await;
                true
            }
        }
    }

    async fn write_key_catalog(
        &self,
        site_id: &str,
        keys: Vec<ApiKeyBinding>,
        groups: Vec<BindableGroup>,
        truncated: bool,
        list_error: Option<String>,
        supported: bool,
    ) {
        let mut map = self.snapshots.write().await;
        let Some(admin) = map.get_mut(site_id).and_then(|s| s.admin.as_mut()) else {
            return;
        };
        admin.api_keys = keys;
        admin.bindable_groups = groups;
        admin.keys_truncated = truncated;
        admin.key_list_error = list_error;
        admin.key_switch_supported = supported;
    }

    async fn set_key_error(&self, site_id: &str, key_id: i64, error: String) {
        let mut map = self.snapshots.write().await;
        let Some(admin) = map.get_mut(site_id).and_then(|s| s.admin.as_mut()) else {
            return;
        };
        if let Some(key) = admin.api_keys.iter_mut().find(|k| k.id == key_id) {
            key.switch_error = Some(error);
        }
    }

    /// The write already succeeded. Move the local row when the follow-up list fails.
    async fn apply_local_group(&self, site_id: &str, key_id: i64, group_id: i64) {
        let mut map = self.snapshots.write().await;
        let Some(admin) = map.get_mut(site_id).and_then(|s| s.admin.as_mut()) else {
            return;
        };
        let name = if group_id <= 0 {
            None
        } else {
            admin
                .bindable_groups
                .iter()
                .find(|g| g.id == group_id)
                .map(|g| g.name.clone())
        };
        if let Some(key) = admin.api_keys.iter_mut().find(|k| k.id == key_id) {
            if group_id <= 0 {
                key.group_id = None;
                key.group_name = None;
            } else {
                key.group_id = Some(group_id);
                key.group_name = name.or_else(|| key.group_name.clone());
            }
            key.switch_error = None;
        }
    }

    async fn mark_switch_unsupported(&self, site_id: &str) {
        let mut map = self.snapshots.write().await;
        if let Some(admin) = map.get_mut(site_id).and_then(|s| s.admin.as_mut()) {
            admin.key_switch_supported = false;
        }
    }
}

async fn refresh_one(client: &Sub2Client, site: SiteConfig) -> SiteSnapshot {
    let public = Store::to_public(&site);
    match site.role {
        SiteRole::User => {
            let api_key = Store::get_secret(&site.id, "api_key").unwrap_or_default();
            let user = if api_key.is_empty() {
                crate::models::UserSnapshot {
                    site_id: site.id.clone(),
                    mode: "error".into(),
                    unit: "USD".into(),
                    balance: None,
                    remaining: None,
                    plan_name: None,
                    is_valid: false,
                    status: Some("error".into()),
                    today: None,
                    total: None,
                    rate_limits: vec![],
                    subscription: None,
                    rpm: None,
                    today_cost: None,
                    month_cost: None,
                    updated_at: Utc::now().to_rfc3339(),
                    error: Some("未配置 API Key".into()),
                }
            } else {
                client
                    .fetch_user_usage(&site.base_url, &api_key, &site.id)
                    .await
            };
            SiteSnapshot {
                site: public,
                user: Some(user),
                admin: None,
                delta: None,
            }
        }
        SiteRole::Admin => {
            let admin = client
                .fetch_admin(&site.id, &site.base_url, site.email.as_deref())
                .await;
            SiteSnapshot {
                site: public,
                user: None,
                admin: Some(admin),
                delta: None,
            }
        }
    }
}

async fn update_tray_tooltip(app: &AppHandle, state: &AppState) {
    let view = state.view().await;
    let mut parts = Vec::new();
    let mut problems = 0u32;
    for s in &view.snapshots {
        if let Some(u) = &s.user {
            if let Some(err) = &u.error {
                problems += 1;
                parts.push(format!("{}: err", s.site.name));
                let _ = err;
            } else if let Some(bal) = u.balance.or(u.remaining) {
                parts.push(format!("{}: ${:.2}", s.site.name, bal));
                if bal >= 0.0 && bal <= view.settings.critical_balance_usd {
                    problems += 1;
                } else if bal >= 0.0 && bal <= view.settings.warn_balance_usd {
                    problems += 1;
                }
            }
        }
        if let Some(a) = &s.admin {
            if a.error.is_some()
                || a.error_accounts > 0
                || a.available_accounts <= view.settings.critical_available_count
            {
                problems += 1;
            }
            parts.push(format!(
                "{}: {}/{}/{}",
                s.site.name, a.available_accounts, a.error_accounts, a.total_accounts
            ));
        }
    }
    let tooltip = if parts.is_empty() {
        "Sub2Viewer — 未配置站点".to_string()
    } else if problems > 0 {
        format!("Sub2Viewer ⚠ {}\n{}", problems, parts.join(" · "))
    } else {
        format!("Sub2Viewer\n{}", parts.join(" · "))
    };

    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(tooltip));
    }
    crate::tray::apply_status_title(app, &view);
}

pub fn schedule_tray_refresh(app: &AppHandle) {
    let state = app.state::<Arc<AppState>>().inner().clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        update_tray_tooltip(&app, &state).await;
    });
}

fn notify(app: &AppHandle, body: &str) {
    use tauri_plugin_notification::NotificationExt;
    if let Err(err) = app
        .notification()
        .builder()
        .title("Sub2Viewer")
        .body(body)
        .show()
    {
        log::warn!("notification failed: {err}");
    }
}

pub fn spawn_poller(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        // initial refresh shortly after start
        tokio::time::sleep(std::time::Duration::from_millis(800)).await;
        state.refresh_all(&app).await;

        loop {
            let secs = {
                let s = state.settings.read().await;
                s.refresh_interval_secs.max(15)
            };
            tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
            state.refresh_all(&app).await;
        }
    });
}
