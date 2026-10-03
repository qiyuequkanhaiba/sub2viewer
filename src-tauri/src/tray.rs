use crate::models::{AppStateView, SiteRole};
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
    pub groups: Vec<(i64, String)>,
    pub can_switch: bool,
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
        let groups: Vec<(i64, String)> = admin
            .bindable_groups
            .iter()
            .map(|g| (g.id, g.name.clone()))
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
            .map(|(id, name)| format!("{id}:{name}"))
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
    for (group_id, name) in &key.groups {
        let item = CheckMenuItem::with_id(
            app,
            format!("kg|{}|{}|{group_id}", key.site_id, key.key_id),
            truncate_chars(name, 48),
            key.can_switch,
            key.group_id == *group_id,
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
                    (2, "ChatGpt(free号池)".into()),
                    (3, "ChatGpt(team号池)".into()),
                ],
                can_switch: true,
            }],
        };
        let before = menu_signature(&model);
        model.keys[0].group_id = 3;
        model.keys[0].group_name = "ChatGpt(team号池)".into();
        assert_ne!(before, menu_signature(&model));
    }
}
