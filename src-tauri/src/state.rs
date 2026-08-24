use crate::client::Sub2Client;
use crate::models::{
    AppSettings, AppSettingsPublic, AppStateView, SiteConfig, SitePublic, SiteRole, SiteSnapshot,
    SiteUpsert,
};
use crate::store::Store;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::sync::RwLock;
use uuid::Uuid;

pub struct AppState {
    pub store: Store,
    pub client: Sub2Client,
    pub settings: RwLock<AppSettings>,
    pub snapshots: RwLock<HashMap<String, SiteSnapshot>>,
    pub last_refresh_at: RwLock<Option<String>>,
    pub refreshing: RwLock<bool>,
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
        })
    }

    pub async fn public_settings(&self) -> AppSettingsPublic {
        let settings = self.settings.read().await;
        AppSettingsPublic {
            refresh_interval_secs: settings.refresh_interval_secs,
            low_balance_threshold: settings.low_balance_threshold,
            warn_balance_usd: settings.warn_balance_usd,
            critical_balance_usd: settings.critical_balance_usd.max(settings.low_balance_threshold),
            warn_health_pct: settings.warn_health_pct,
            critical_health_pct: settings.critical_health_pct,
            warn_available_count: settings.warn_available_count,
            critical_available_count: settings.critical_available_count,
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
            handles.push(tokio::spawn(async move {
                refresh_one(&client, site).await
            }));
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
            let enabled_ids: std::collections::HashSet<_> =
                new_map.keys().cloned().collect();
            map.retain(|k, _| enabled_ids.contains(k));
            for (k, v) in new_map {
                map.insert(k, v);
            }
        }

        *self.last_refresh_at.write().await = Some(Utc::now().to_rfc3339());
        *self.refreshing.write().await = false;
        let _ = app.emit("refreshing", false);
        self.emit_state(app).await;
        update_tray_tooltip(app, self).await;
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
                "{}: {}/{} ok",
                s.site.name, a.available_accounts, a.total_accounts
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
