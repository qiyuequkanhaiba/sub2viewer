mod alerts;
mod client;
mod commands;
mod history;
mod hud;
mod models;
mod state;
mod store;
mod tray;

use state::{spawn_poller, AppState};
use std::sync::Arc;
use tauri::{
    image::Image,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, Position,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = env_logger::try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            // Menu-bar style: no Dock icon.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let state = Arc::new(AppState::new().map_err(|e| {
                log::error!("failed to init state: {e}");
                e
            })?);
            app.manage(state.clone());

            // Right-click tray menu is the only chrome. Key submenus are filled after refresh.
            let menu = tray::build_tray_menu(app, &tray::TrayModel::default())?;

            let icon = app.default_window_icon().cloned().unwrap_or_else(|| {
                Image::from_bytes(include_bytes!("../icons/32x32.png")).expect("icon")
            });

            let _tray = TrayIconBuilder::with_id("main")
                .icon(icon)
                .menu(&menu)
                .show_menu_on_left_click(false)
                .tooltip("Sub2Viewer")
                .on_menu_event({
                    let state = state.clone();
                    move |app, event| match event.id.as_ref() {
                        "show" => {
                            show_panel_window(app);
                        }
                        "hide" => hide_panel_window(app),
                        "refresh" => {
                            let app2 = app.clone();
                            let st = state.clone();
                            tauri::async_runtime::spawn(async move {
                                st.refresh_all(&app2).await;
                            });
                        }
                        "settings" => {
                            let _ = commands::open_settings_window(app.clone());
                        }
                        "reset_pos" => {
                            let st = state.clone();
                            let app2 = app.clone();
                            tauri::async_runtime::spawn(async move {
                                st.clear_panel_position().await;
                                if let Some(win) = app2.get_webview_window("panel") {
                                    position_at_notch(&win);
                                    let _ = win.show();
                                    crate::tray::apply_status_title(&app2, &st.view().await);
                                }
                            });
                        }
                        "quit" => {
                            app.exit(0);
                        }
                        other => {
                            if let Some((site_id, key_id, group_id)) =
                                tray::parse_key_group_menu_id(other)
                            {
                                let app2 = app.clone();
                                let st = state.clone();
                                tauri::async_runtime::spawn(async move {
                                    st.switch_key_group(&app2, site_id, key_id, group_id).await;
                                });
                            }
                        }
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        toggle_panel_window(app);
                    }
                })
                .build(app)?;

            // Floating HUD: start hidden. The island is anchored to
            // the notch, so window positions are not persisted: the
            // pre-island Moved handler saved every drag/programmatic
            // move to settings.panel_x/panel_y and restore_or_place
            // put the window back there, which on machines with a
            // saved tray-panel position left the island hanging below
            // the menu bar instead of on the notch.
            hud::apply_clear(app.handle());
            hud::apply_island_window(app.handle());
            hud::spawn_click_through_guard(app.handle().clone());
            if let Some(panel) = app.get_webview_window("panel") {
                let _ = panel.hide();
            }

            let launch_app = app.handle().clone();
            let launch_state = state.clone();
            tauri::async_runtime::spawn(async move {
                let enabled = launch_state.settings.read().await.launch_at_login;
                if let Err(err) = commands::sync_launch_at_login(&launch_app, enabled) {
                    log::warn!("launch at login sync failed: {err}");
                }
            });

            spawn_poller(app.handle().clone(), state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::refresh_now,
            commands::upsert_site,
            commands::delete_site,
            commands::update_settings,
            commands::admin_login,
            commands::admin_login_2fa,
            commands::show_panel,
            commands::hide_panel,
            commands::open_settings_window,
            commands::get_notch_geometry,
            commands::set_pill_rect,
        ])
        .run(tauri::generate_context!())
        .expect("error while running sub2viewer");
}

fn show_panel_window(app: &tauri::AppHandle) {
    hud::apply_clear(app);
    if let Some(win) = app.get_webview_window("panel") {
        restore_or_place(&win, app);
        // No set_focus: the island must not pull focus from the
        // frontmost app.
        let _ = win.show();
    }
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_title(None::<&str>);
    }
}

fn hide_panel_window(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("panel") {
        let _ = win.hide();
    }
    state::schedule_tray_refresh(app);
}

fn toggle_panel_window(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("panel") {
        if win.is_visible().unwrap_or(false) {
            hide_panel_window(app);
        } else {
            show_panel_window(app);
        }
    }
}

fn restore_or_place(win: &tauri::WebviewWindow, _app: &tauri::AppHandle) {
    // The island is anchored to the notch by design: always place it
    // there. A saved panel_x/panel_y (left over from the pre-island
    // tray panel, or from a drag) would leave it hanging below the
    // menu bar, off-notch, so the saved position is never restored
    // for this window and is no longer written either (see setup).
    position_at_notch(win);
}

fn position_at_notch(win: &tauri::WebviewWindow) {
    // Anchor the fixed-size island window at the top-center of the
    // primary screen. The capsule inside hugs the top edge: it covers
    // the hardware notch on notched displays and forms a virtual island
    // on notch-less ones (e.g. a Mac mini with an external monitor).
    let monitor = win
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| win.current_monitor().ok().flatten());
    if let Some(monitor) = monitor {
        let scale = monitor.scale_factor();
        let mon_pos = monitor.position();
        let mon_size = monitor.size();
        let win_size = win
            .outer_size()
            .unwrap_or(tauri::PhysicalSize::new(360, 580));
        let x = mon_pos.x as f64 / scale
            + (mon_size.width as f64 / scale - win_size.width as f64 / scale) / 2.0;
        let y = mon_pos.y as f64 / scale;
        let _ = win.set_position(Position::Logical(tauri::LogicalPosition { x, y }));
    }
}
