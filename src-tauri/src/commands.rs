use crate::client::LoginOutcome;
use crate::models::{
    AppStateView, LoginResult, SettingsUpdate, SitePublic, SiteUpsert,
};
use crate::state::AppState;
use crate::store::Store;
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};

#[tauri::command]
pub async fn get_state(state: State<'_, Arc<AppState>>) -> Result<AppStateView, String> {
    Ok(state.view().await)
}

#[tauri::command]
pub async fn refresh_now(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<AppStateView, String> {
    state.refresh_all(&app).await;
    Ok(state.view().await)
}

#[tauri::command]
pub async fn upsert_site(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    input: SiteUpsert,
) -> Result<SitePublic, String> {
    let site = state.upsert_site(input).await?;
    state.emit_state(&app).await;
    // kick a refresh in background
    let app2 = app.clone();
    let st = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        st.refresh_all(&app2).await;
    });
    Ok(site)
}

#[tauri::command]
pub async fn delete_site(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    site_id: String,
) -> Result<(), String> {
    state.delete_site(&site_id).await?;
    state.emit_state(&app).await;
    Ok(())
}

#[tauri::command]
pub async fn update_settings(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    input: SettingsUpdate,
) -> Result<(), String> {
    state
        .update_settings(input.refresh_interval_secs, input.low_balance_threshold)
        .await?;
    state.emit_state(&app).await;
    Ok(())
}

#[tauri::command]
pub async fn admin_login(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    site_id: String,
    email: String,
    password: String,
) -> Result<LoginResult, String> {
    let (base_url, _) = {
        let settings = state.settings.read().await;
        let site = settings
            .sites
            .iter()
            .find(|s| s.id == site_id)
            .ok_or_else(|| "站点不存在".to_string())?;
        (site.base_url.clone(), site.role.clone())
    };

    match state
        .client
        .login(&base_url, email.trim(), &password)
        .await
        .map_err(|e| e.to_string())?
    {
        LoginOutcome::Requires2FA {
            temp_token,
            user_email_masked,
        } => {
            // stash password for after 2FA
            Store::set_secret(&site_id, "password", &password).map_err(|e| e.to_string())?;
            // update email on site
            {
                let mut settings = state.settings.write().await;
                if let Some(site) = settings.sites.iter_mut().find(|s| s.id == site_id) {
                    site.email = Some(email.trim().to_string());
                    state
                        .store
                        .save_settings(&settings)
                        .map_err(|e| e.to_string())?;
                }
            }
            Ok(LoginResult {
                requires_2fa: true,
                temp_token: Some(temp_token),
                user_email_masked,
                role: None,
                message: "需要二次验证".into(),
            })
        }
        LoginOutcome::Success {
            access_token,
            refresh_token,
            role,
        } => {
            Store::set_secret(&site_id, "password", &password).map_err(|e| e.to_string())?;
            Store::set_secret(&site_id, "access_token", &access_token)
                .map_err(|e| e.to_string())?;
            if let Some(r) = refresh_token {
                Store::set_secret(&site_id, "refresh_token", &r).map_err(|e| e.to_string())?;
            }
            {
                let mut settings = state.settings.write().await;
                if let Some(site) = settings.sites.iter_mut().find(|s| s.id == site_id) {
                    site.email = Some(email.trim().to_string());
                    state
                        .store
                        .save_settings(&settings)
                        .map_err(|e| e.to_string())?;
                }
            }
            state.emit_state(&app).await;
            let app2 = app.clone();
            let st = state.inner().clone();
            tauri::async_runtime::spawn(async move {
                st.refresh_all(&app2).await;
            });
            Ok(LoginResult {
                requires_2fa: false,
                temp_token: None,
                user_email_masked: None,
                role,
                message: "登录成功".into(),
            })
        }
    }
}

#[tauri::command]
pub async fn admin_login_2fa(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    site_id: String,
    temp_token: String,
    totp_code: String,
) -> Result<LoginResult, String> {
    let base_url = {
        let settings = state.settings.read().await;
        settings
            .sites
            .iter()
            .find(|s| s.id == site_id)
            .map(|s| s.base_url.clone())
            .ok_or_else(|| "站点不存在".to_string())?
    };

    match state
        .client
        .login_2fa(&base_url, &temp_token, totp_code.trim())
        .await
        .map_err(|e| e.to_string())?
    {
        LoginOutcome::Success {
            access_token,
            refresh_token,
            role,
        } => {
            Store::set_secret(&site_id, "access_token", &access_token)
                .map_err(|e| e.to_string())?;
            if let Some(r) = refresh_token {
                Store::set_secret(&site_id, "refresh_token", &r).map_err(|e| e.to_string())?;
            }
            state.emit_state(&app).await;
            let app2 = app.clone();
            let st = state.inner().clone();
            tauri::async_runtime::spawn(async move {
                st.refresh_all(&app2).await;
            });
            Ok(LoginResult {
                requires_2fa: false,
                temp_token: None,
                user_email_masked: None,
                role,
                message: "2FA 登录成功".into(),
            })
        }
        LoginOutcome::Requires2FA { .. } => Err("2FA 仍未完成".into()),
    }
}

#[tauri::command]
pub fn show_panel(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("panel") {
        let _ = win.show();
        let _ = win.set_focus();
    }
    Ok(())
}

#[tauri::command]
pub fn hide_panel(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("panel") {
        let _ = win.hide();
    }
    Ok(())
}

#[tauri::command]
pub fn open_settings_window(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("settings") {
        let _ = win.show();
        let _ = win.set_focus();
        return Ok(());
    }
    // create settings window if missing
    tauri::WebviewWindowBuilder::new(
        &app,
        "settings",
        tauri::WebviewUrl::App("/?view=settings".into()),
    )
    .title("Sub2Viewer 设置")
    .inner_size(920.0, 640.0)
    .min_inner_size(820.0, 540.0)
    .resizable(true)
    .decorations(true)
    .skip_taskbar(false)
    .always_on_top(false)
    .build()
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn set_hud_radius(app: AppHandle, radius: f64) -> Result<(), String> {
    crate::hud::set_corner_radius(&app, radius);
    Ok(())
}
