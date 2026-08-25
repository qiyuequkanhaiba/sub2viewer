use crate::models::{AppSettings, SiteConfig, SitePublic, SiteRole};
use serde_json::{Map, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use thiserror::Error;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Local secret vault. Keychain is avoided because unsigned / accessory
/// (menu-bar) apps routinely fail the macOS "allow access" dialog even after
/// the user types the login password.
fn data_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("sub2viewer")
}

fn secrets_path() -> PathBuf {
    data_dir().join("secrets.json")
}

fn secrets_lock() -> &'static Mutex<Map<String, Value>> {
    static LOCK: OnceLock<Mutex<Map<String, Value>>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(load_secrets_file()))
}

fn load_secrets_file() -> Map<String, Value> {
    let path = secrets_path();
    let Ok(raw) = fs::read_to_string(&path) else {
        return Map::new();
    };
    serde_json::from_str::<Value>(&raw)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn persist_secrets(map: &Map<String, Value>) -> Result<(), StoreError> {
    let dir = data_dir();
    fs::create_dir_all(&dir)?;
    let path = secrets_path();
    let raw = serde_json::to_string_pretty(&Value::Object(map.clone()))?;
    fs::write(&path, raw)?;
    #[cfg(unix)]
    {
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn secret_key(site_id: &str, kind: &str) -> String {
    format!("site.{site_id}.{kind}")
}

pub struct Store {
    config_path: PathBuf,
}

impl Store {
    pub fn new() -> Result<Self, StoreError> {
        let dir = data_dir();
        fs::create_dir_all(&dir)?;
        Ok(Self {
            config_path: dir.join("config.json"),
        })
    }

    pub fn load_settings(&self) -> Result<AppSettings, StoreError> {
        if !self.config_path.exists() {
            return Ok(AppSettings::default());
        }
        let raw = fs::read_to_string(&self.config_path)?;
        let settings: AppSettings = serde_json::from_str(&raw)?;
        Ok(settings)
    }

    pub fn save_settings(&self, settings: &AppSettings) -> Result<(), StoreError> {
        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(settings)?;
        fs::write(&self.config_path, raw)?;
        Ok(())
    }

    pub fn set_secret(site_id: &str, kind: &str, value: &str) -> Result<(), StoreError> {
        let key = secret_key(site_id, kind);
        let mut map = secrets_lock().lock().unwrap_or_else(|e| e.into_inner());
        if value.is_empty() {
            map.remove(&key);
        } else {
            map.insert(key, Value::String(value.to_string()));
        }
        persist_secrets(&map)
    }

    pub fn get_secret(site_id: &str, kind: &str) -> Option<String> {
        let key = secret_key(site_id, kind);
        let map = secrets_lock().lock().unwrap_or_else(|e| e.into_inner());
        map.get(&key)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    }

    pub fn delete_secret(site_id: &str, kind: &str) {
        let _ = Self::set_secret(site_id, kind, "");
    }

    pub fn delete_all_secrets(site_id: &str) {
        for kind in ["api_key", "password", "access_token", "refresh_token"] {
            Self::delete_secret(site_id, kind);
        }
    }

    pub fn has_secret(site_id: &str, kind: &str) -> bool {
        Self::get_secret(site_id, kind).is_some()
    }

    pub fn to_public(site: &SiteConfig) -> SitePublic {
        SitePublic {
            id: site.id.clone(),
            name: site.name.clone(),
            base_url: site.base_url.clone(),
            role: site.role.clone(),
            api_key_label: site.api_key_label.clone(),
            email: site.email.clone(),
            enabled: site.enabled,
            has_api_key: Self::has_secret(&site.id, "api_key"),
            has_password: Self::has_secret(&site.id, "password"),
            has_token: Self::has_secret(&site.id, "access_token"),
        }
    }

    pub fn normalize_base_url(url: &str) -> String {
        let mut u = url.trim().trim_end_matches('/').to_string();
        if u.ends_with("/api/v1") {
            u = u
                .trim_end_matches("/api/v1")
                .trim_end_matches('/')
                .to_string();
        }
        u
    }

    pub fn validate_site(site: &SiteConfig) -> Result<(), String> {
        if site.name.trim().is_empty() {
            return Err("站点名称不能为空".into());
        }
        if site.base_url.trim().is_empty() {
            return Err("站点 URL 不能为空".into());
        }
        let url = url::Url::parse(&site.base_url).map_err(|_| "站点 URL 格式无效".to_string())?;
        if url.scheme() != "http" && url.scheme() != "https" {
            return Err("站点 URL 仅支持 http/https".into());
        }
        if matches!(site.role, SiteRole::Admin)
            && site
                .email
                .as_ref()
                .map(|e| e.trim().is_empty())
                .unwrap_or(true)
        {
            return Err("管理员模式需要填写邮箱".into());
        }
        Ok(())
    }
}
