use crate::models::{AppStateView, GroupHealth, SiteRole};
use std::sync::Mutex;
use tauri::menu::{CheckMenuItem, Menu, MenuBuilder, MenuItem, Submenu, SubmenuBuilder};
use tauri::{AppHandle, Manager, Wry};

const TRAY_ID: &str = "main";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayKey {
    pub site_id: String,
    pub site_name: String,
    pub key_id: i64,
    pub key_name: String,
    pub status: String,
    /// 0 means the key is not bound to a group.
    pub group_id: i64,
    pub group_name: String,
    pub groups: Vec<TrayGroup>,
    pub can_switch: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayGroup {
    pub id: i64,
    pub name: String,
    pub available: Option<i64>,
    pub rate_limited: Option<i64>,
    pub error: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayNotice {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrayModel {
    pub prefix_site: bool,
    pub keys: Vec<TrayKey>,
    pub notices: Vec<TrayNotice>,
}

pub fn model_from_view(view: &AppStateView) -> TrayModel {
    let mut keys = Vec::new();
    let mut notices = Vec::new();
    let mut site_ids = Vec::new();
    for snap in &view.snapshots {
        if snap.site.role != SiteRole::Admin {
            continue;
        }
        let Some(admin) = &snap.admin else {
            continue;
        };
        let groups: Vec<TrayGroup> = admin
            .bindable_groups
            .iter()
            .map(|g| {
                let health = health_for(&admin.groups, g.id);
                TrayGroup {
                    id: g.id,
                    name: g.name.clone(),
                    available: health.map(|item| item.available),
                    rate_limited: health.map(|item| item.rate_limited),
                    error: health.map(|item| item.error),
                }
            })
            .collect();
        let groups_missing = groups.is_empty() && admin.key_list_error.is_some();
        let can_switch = admin.key_switch_supported && !groups_missing;
        if admin.api_keys.is_empty() {
            if admin.key_list_error.is_some() {
                site_ids.push(snap.site.id.clone());
                notices.push(TrayNotice {
                    id: format!("kgnote|{}", snap.site.id),
                    label: truncate_chars(&format!("{} · 密钥列表失败", snap.site.name), 48),
                });
            }
            continue;
        }
        site_ids.push(snap.site.id.clone());
        for key in &admin.api_keys {
            keys.push(TrayKey {
                site_id: snap.site.id.clone(),
                site_name: snap.site.name.clone(),
                key_id: key.id,
                key_name: key.name.clone(),
                status: key.status.clone(),
                group_id: key.group_id.unwrap_or(0),
                group_name: key
                    .group_name
                    .clone()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "未分组".into()),
                groups: groups.clone(),
                can_switch,
            });
        }
    }
    site_ids.sort();
    site_ids.dedup();
    TrayModel {
        prefix_site: site_ids.len() > 1,
        keys,
        notices,
    }
}

pub fn menu_signature(model: &TrayModel) -> String {
    let mut parts = Vec::new();
    parts.push(if model.prefix_site { "1" } else { "0" }.to_string());
    for notice in &model.notices {
        parts.push(format!("n:{}:{}", notice.id, notice.label));
    }
    for key in &model.keys {
        let groups = key
            .groups
            .iter()
            .map(|group| {
                format!(
                    "{}:{}:{}:{}:{}",
                    group.id,
                    group.name,
                    group
                        .available
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "-".into()),
                    group
                        .rate_limited
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "-".into()),
                    group
                        .error
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "-".into())
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        parts.push(format!(
            "k:{}:{}:{}:{}:{}:{}:{}:{}",
            key.site_id,
            key.key_id,
            key.key_name,
            key.status,
            key.group_id,
            key.group_name,
            key.can_switch,
            groups
        ));
    }
    parts.join("\n")
}

pub fn parse_key_group_menu_id(id: &str) -> Option<(String, i64, i64)> {
    let rest = id.strip_prefix("kg|")?;
    let mut parts = rest.split('|');
    let site = parts.next()?.to_string();
    if site.is_empty() {
        return None;
    }
    let key_id = parts.next()?.parse::<i64>().ok()?;
    let group_id = parts.next()?.parse::<i64>().ok()?;
    if parts.next().is_some() || key_id <= 0 || group_id < 0 {
        return None;
    }
    Some((site, key_id, group_id))
}

pub fn build_tray_menu(app: &impl Manager<Wry>, model: &TrayModel) -> tauri::Result<Menu<Wry>> {
    let show_i = MenuItem::with_id(app, "show", "显示监控", true, None::<&str>)?;
    let hide_i = MenuItem::with_id(app, "hide", "隐藏监控", true, None::<&str>)?;
    let refresh_i = MenuItem::with_id(app, "refresh", "立即刷新", true, None::<&str>)?;
    let settings_i = MenuItem::with_id(app, "settings", "设置…", true, None::<&str>)?;
    let reset_pos_i = MenuItem::with_id(app, "reset_pos", "重置窗口位置", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;

    let mut builder = MenuBuilder::new(app)
        .item(&show_i)
        .item(&hide_i)
        .separator()
        .item(&refresh_i);

    let mut notices = Vec::new();
    for notice in &model.notices {
        notices.push(MenuItem::with_id(
            app,
            &notice.id,
            &notice.label,
            false,
            None::<&str>,
        )?);
    }
    let mut submenus: Vec<Submenu<Wry>> = Vec::new();
    for key in &model.keys {
        submenus.push(build_key_submenu(app, key, model.prefix_site)?);
    }
    if !notices.is_empty() || !submenus.is_empty() {
        builder = builder.separator();
        for notice in &notices {
            builder = builder.item(notice);
        }
        for submenu in &submenus {
            builder = builder.item(submenu);
        }
        builder = builder.separator();
    }

    builder
        .item(&settings_i)
        .item(&reset_pos_i)
        .separator()
        .item(&quit_i)
        .build()
}

fn build_key_submenu(
    app: &impl Manager<Wry>,
    key: &TrayKey,
    prefix_site: bool,
) -> tauri::Result<Submenu<Wry>> {
    let mut label = if prefix_site {
        format!(
            "{} · {} · {}",
            key.site_name,
            display_key_name(key),
            key.group_name
        )
    } else {
        format!("{} · {}", display_key_name(key), key.group_name)
    };
    label = truncate_chars(&label, 64);
    let mut builder = SubmenuBuilder::with_id(
        app,
        &format!("kgsub|{}|{}", key.site_id, key.key_id),
        &label,
    );
    if !key.can_switch {
        let blocked = MenuItem::with_id(
            app,
            &format!("kgblocked|{}|{}", key.site_id, key.key_id),
            "当前站点不能切换分组",
            false,
            None::<&str>,
        )?;
        builder = builder.item(&blocked);
    }
    for group in &key.groups {
        let item = CheckMenuItem::with_id(
            app,
            format!("kg|{}|{}|{}", key.site_id, key.key_id, group.id),
            group_menu_label(group),
            key.can_switch,
            key.group_id == group.id,
            None::<&str>,
        )?;
        builder = builder.item(&item);
    }
    let unbind = CheckMenuItem::with_id(
        app,
        format!("kg|{}|{}|0", key.site_id, key.key_id),
        "解除分组",
        key.can_switch,
        key.group_id == 0,
        None::<&str>,
    )?;
    if !key.groups.is_empty() {
        builder = builder.separator();
    }
    builder.item(&unbind).build()
}

fn health_for(groups: &[GroupHealth], id: i64) -> Option<&GroupHealth> {
    groups.iter().find(|group| group.group_id == id)
}

pub fn group_menu_label(group: &TrayGroup) -> String {
    let (Some(available), Some(rate_limited), Some(error)) =
        (group.available, group.rate_limited, group.error)
    else {
        return truncate_chars(&group.name, 48);
    };
    let suffix = format!(" · 正常{available} 限流{rate_limited} 异常{error}");
    let max = 72usize;
    let suffix_len = suffix.chars().count();
    if suffix_len >= max {
        return truncate_chars(&suffix, max);
    }
    let name = truncate_chars(&group.name, max - suffix_len);
    format!("{name}{suffix}")
}

/// Menu-bar title. Balance uses the same sum as the HUD; accounts are 正常/错误/总量.
pub fn status_title(view: &AppStateView) -> String {
    let mut balance = 0.0;
    let mut has_balance = false;
    let mut unlimited = false;
    let mut available = 0i64;
    let mut errors = 0i64;
    let mut total = 0i64;
    let mut has_admin = false;
    for snap in &view.snapshots {
        if let Some(user) = &snap.user {
            if user.error.is_some() && user.remaining.is_none() && user.balance.is_none() {
                continue;
            }
            match user.remaining.or(user.balance) {
                Some(value) if value < 0.0 => unlimited = true,
                Some(value) => {
                    balance += value;
                    has_balance = true;
                }
                None => {}
            }
        }
        if let Some(admin) = &snap.admin {
            if admin.error.is_some() && admin.total_accounts <= 0 {
                continue;
            }
            has_admin = true;
            available += admin.available_accounts;
            errors += admin.error_accounts;
            total += admin.total_accounts;
        }
    }
    let money = if unlimited && !has_balance {
        Some("∞".to_string())
    } else if has_balance {
        Some(compact_usd(balance))
    } else {
        None
    };
    let accounts = has_admin.then(|| format!("{available}/{errors}/{total}"));
    match (money, accounts) {
        (Some(money), Some(accounts)) => format!("{money} · {accounts}"),
        (Some(money), None) => money,
        (None, Some(accounts)) => accounts,
        (None, None) => String::new(),
    }
}

fn compact_usd(value: f64) -> String {
    if value >= 100.0 || (value - value.round()).abs() < 0.001 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}

pub fn panel_is_open(app: &AppHandle) -> bool {
    app.get_webview_window("panel")
        .and_then(|win| win.is_visible().ok())
        .unwrap_or(false)
}

/// The menu-bar title repeats the HUD, so it stays blank while the panel is open.
pub fn apply_status_title(app: &AppHandle, view: &AppStateView) {
    let Some(tray) = app.tray_by_id("main") else {
        return;
    };
    if panel_is_open(app) {
        let _ = tray.set_title(None::<&str>);
        return;
    }
    let title = status_title(view);
    let _ = if title.is_empty() {
        tray.set_title(None::<&str>)
    } else {
        tray.set_title(Some(title))
    };
}

fn display_key_name(key: &TrayKey) -> String {
    if key.status != "active" && !key.status.is_empty() {
        format!("{}（停用）", key.key_name)
    } else {
        key.key_name.clone()
    }
}

pub fn apply_tray_menu(app: &AppHandle, view: &AppStateView) {
    let model = model_from_view(view);
    let signature = menu_signature(&model);
    let mut last = LAST_SIGNATURE.lock().unwrap_or_else(|p| p.into_inner());
    if *last == signature {
        return;
    }
    match build_tray_menu(app, &model) {
        Ok(menu) => {
            if let Some(tray) = app.tray_by_id(TRAY_ID) {
                if tray.set_menu(Some(menu)).is_ok() {
                    *last = signature;
                }
            }
        }
        Err(e) => log::error!("rebuild tray menu: {e}"),
    }
}

static LAST_SIGNATURE: Mutex<String> = Mutex::new(String::new());

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_id_round_trip() {
        let id = "kg|1aa9ed8d-a090-4183-9454-a4c56bb56a2e|7|3";
        let (site, key, group) = parse_key_group_menu_id(id).unwrap();
        assert_eq!(site, "1aa9ed8d-a090-4183-9454-a4c56bb56a2e");
        assert_eq!(key, 7);
        assert_eq!(group, 3);
        assert!(parse_key_group_menu_id("kg|site|0|1").is_none());
        assert!(parse_key_group_menu_id("kg|site|4|-1").is_none());
        assert!(parse_key_group_menu_id("show").is_none());
    }

    #[test]
    fn signature_changes_when_group_changes() {
        let mut model = TrayModel {
            prefix_site: false,
            notices: vec![],
            keys: vec![TrayKey {
                site_id: "s".into(),
                site_name: "Quyue".into(),
                key_id: 7,
                key_name: "Gpt-free".into(),
                status: "active".into(),
                group_id: 2,
                group_name: "ChatGpt(free号池)".into(),
                groups: vec![
                    TrayGroup {
                        id: 2,
                        name: "ChatGpt(free号池)".into(),
                        available: Some(4),
                        rate_limited: Some(0),
                        error: Some(1),
                    },
                    TrayGroup {
                        id: 3,
                        name: "ChatGpt(team号池)".into(),
                        available: Some(12),
                        rate_limited: Some(3),
                        error: Some(1),
                    },
                ],
                can_switch: true,
            }],
        };
        let before = menu_signature(&model);
        model.keys[0].group_id = 3;
        model.keys[0].group_name = "ChatGpt(team号池)".into();
        assert_ne!(before, menu_signature(&model));
        let health_before = menu_signature(&model);
        model.keys[0].groups[1].available = Some(1);
        assert_ne!(health_before, menu_signature(&model));
    }

    #[test]
    fn group_label_keeps_pool_health() {
        let label = group_menu_label(&TrayGroup {
            id: 3,
            name: "ChatGpt(team号池)".into(),
            available: Some(12),
            rate_limited: Some(3),
            error: Some(1),
        });
        assert_eq!(label, "ChatGpt(team号池) · 正常12 限流3 异常1");
        let bare = group_menu_label(&TrayGroup {
            id: 1,
            name: "未统计".into(),
            available: None,
            rate_limited: None,
            error: None,
        });
        assert_eq!(bare, "未统计");
    }

    #[test]
    fn status_title_matches_the_hud_shape() {
        let view = sample_view(Some(12.4), Some((18, 2, 40)));
        assert_eq!(status_title(&view), "12.40 · 18/2/40");
        let money = sample_view(Some(12.0), None);
        assert_eq!(status_title(&money), "12");
        let accounts = sample_view(None, Some((3, 1, 8)));
        assert_eq!(status_title(&accounts), "3/1/8");
    }

    fn sample_view(balance: Option<f64>, accounts: Option<(i64, i64, i64)>) -> AppStateView {
        use crate::models::{AdminSnapshot, AppSettingsPublic, SiteSnapshot, UserSnapshot};
        let mut snapshots = Vec::new();
        if let Some(balance) = balance {
            snapshots.push(SiteSnapshot {
                site: site("user", SiteRole::User),
                user: Some(UserSnapshot {
                    site_id: "u".into(),
                    mode: "usage".into(),
                    unit: "USD".into(),
                    balance: Some(balance),
                    remaining: Some(balance),
                    plan_name: None,
                    is_valid: true,
                    status: None,
                    today: None,
                    total: None,
                    rate_limits: vec![],
                    subscription: None,
                    rpm: None,
                    today_cost: None,
                    month_cost: None,
                    updated_at: String::new(),
                    error: None,
                }),
                admin: None,
                delta: None,
            });
        }
        if let Some((available, errors, total)) = accounts {
            snapshots.push(SiteSnapshot {
                site: site("admin", SiteRole::Admin),
                user: None,
                admin: Some(AdminSnapshot {
                    site_id: "a".into(),
                    monitoring_enabled: true,
                    status_breakdown: Default::default(),
                    groups: vec![],
                    total_accounts: total,
                    available_accounts: available,
                    error_accounts: errors,
                    rate_limited_accounts: 0,
                    unschedulable_accounts: 0,
                    issues: vec![],
                    api_keys: vec![],
                    bindable_groups: vec![],
                    key_switch_supported: true,
                    keys_truncated: false,
                    key_list_error: None,
                    today_cost: None,
                    month_cost: None,
                    updated_at: String::new(),
                    error: None,
                }),
                delta: None,
            });
        }
        AppStateView {
            settings: AppSettingsPublic {
                refresh_interval_secs: 60,
                low_balance_threshold: 1.0,
                warn_balance_usd: 5.0,
                critical_balance_usd: 1.0,
                warn_health_pct: 80.0,
                critical_health_pct: 50.0,
                warn_available_count: 5,
                critical_available_count: 2,
                launch_at_login: false,
                sites: vec![],
            },
            snapshots,
            last_refresh_at: None,
            refreshing: false,
        }
    }

    fn site(id: &str, role: SiteRole) -> crate::models::SitePublic {
        crate::models::SitePublic {
            id: id.into(),
            name: id.into(),
            base_url: "https://example.test".into(),
            role,
            api_key_label: None,
            email: None,
            enabled: true,
            has_api_key: false,
            has_password: false,
            has_token: false,
        }
    }
}
