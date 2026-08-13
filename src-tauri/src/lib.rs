mod client;
mod commands;
mod hud;
mod models;
mod state;
mod store;

use state::{spawn_poller, AppState};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, Position, WindowEvent,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = env_logger::try_init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Menu-bar style: no Dock icon.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let state = Arc::new(AppState::new().map_err(|e| {
                log::error!("failed to init state: {e}");
                e
            })?);
            app.manage(state.clone());

            // Right-click tray menu is the only chrome.
            let show_i = MenuItem::with_id(app, "show", "显示监控", true, None::<&str>)?;
            let hide_i = MenuItem::with_id(app, "hide", "隐藏监控", true, None::<&str>)?;
            let refresh_i = MenuItem::with_id(app, "refresh", "立即刷新", true, None::<&str>)?;
            let settings_i = MenuItem::with_id(app, "settings", "设置…", true, None::<&str>)?;
            let reset_pos_i =
                MenuItem::with_id(app, "reset_pos", "重置窗口位置", true, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
            let sep1 = PredefinedMenuItem::separator(app)?;
            let sep2 = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(
                app,
                &[
                    &show_i,
                    &hide_i,
                    &sep1,
                    &refresh_i,
                    &settings_i,
                    &reset_pos_i,
                    &sep2,
                    &quit_i,
                ],
            )?;

            let icon = app
                .default_window_icon()
                .cloned()
                .unwrap_or_else(|| Image::from_bytes(include_bytes!("../icons/32x32.png")).expect("icon"));

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
                        "hide" => {
                            if let Some(win) = app.get_webview_window("panel") {
                                let _ = win.hide();
                            }
                        }
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
                                    position_near_tray(&win);
                                    let _ = win.show();
                                }
                            });
                        }
                        "quit" => {
                            app.exit(0);
                        }
                        _ => {}
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

            // Floating HUD: start hidden; remember drag position.
            hud::apply_clear(app.handle());
            if let Some(panel) = app.get_webview_window("panel") {
                let _ = panel.hide();
                let st = state.clone();
                let win = panel.clone();
                let last_save = std::sync::Mutex::new(Instant::now() - Duration::from_secs(1));
                panel.on_window_event(move |event| {
                    if let WindowEvent::Moved(pos) = event {
                        let now = Instant::now();
                        if let Ok(mut t) = last_save.lock() {
                            if now.duration_since(*t) < Duration::from_millis(200) {
                                return;
                            }
                            *t = now;
                        }
                        let scale = win.scale_factor().unwrap_or(1.0);
                        let x = pos.x as f64 / scale;
                        let y = pos.y as f64 / scale;
                        let st = st.clone();
                        tauri::async_runtime::spawn(async move {
                            st.save_panel_position(x, y).await;
                        });
                    }
                });
            }

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
            commands::set_hud_radius,
        ])
        .run(tauri::generate_context!())
        .expect("error while running sub2viewer");
}

fn show_panel_window(app: &tauri::AppHandle) {
    hud::apply_clear(app);
    if let Some(win) = app.get_webview_window("panel") {
        restore_or_place(&win, app);
        let _ = win.show();
        let _ = win.set_focus();
    }
}

fn toggle_panel_window(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("panel") {
        if win.is_visible().unwrap_or(false) {
            let _ = win.hide();
        } else {
            restore_or_place(&win, app);
            let _ = win.show();
            let _ = win.set_focus();
        }
    }
}

fn restore_or_place(win: &tauri::WebviewWindow, app: &tauri::AppHandle) {
    let state = app.state::<Arc<AppState>>();
    let st = state.inner().clone();
    let win = win.clone();
    tauri::async_runtime::spawn(async move {
        let (x, y) = {
            let s = st.settings.read().await;
            (s.panel_x, s.panel_y)
        };
        match (x, y) {
            (Some(x), Some(y)) => {
                let _ = win.set_position(Position::Logical(tauri::LogicalPosition { x, y }));
            }
            _ => position_near_tray(&win),
        }
    });
}

fn position_near_tray(win: &tauri::WebviewWindow) {
    // Best-effort: place near top-right (menu bar / HUD zone).
    if let Ok(Some(monitor)) = win.current_monitor() {
        let screen = monitor.size();
        let scale = monitor.scale_factor();
        let win_size = win.outer_size().unwrap_or(tauri::PhysicalSize::new(480, 300));
        let x = (screen.width as f64 / scale) - (win_size.width as f64 / scale) - 16.0;
        let y = 36.0;
        let _ = win.set_position(tauri::Position::Logical(tauri::LogicalPosition { x, y }));
    }
}


