use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SiteRole {
    User,
    Admin,
}

impl Default for SiteRole {
    fn default() -> Self {
        Self::User
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteConfig {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub role: SiteRole,
    /// Display label for user API key (not the secret itself).
    #[serde(default)]
    pub api_key_label: Option<String>,
    /// Admin login email (password stored in local secrets file).
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    #[serde(default = "default_refresh")]
    pub refresh_interval_secs: u64,
    #[serde(default = "default_low_balance")]
    pub low_balance_threshold: f64,
    /// Warn when remaining USD is below this (still above critical).
    #[serde(default = "default_warn_balance")]
    pub warn_balance_usd: f64,
    /// Critical / red when remaining USD is below this.
    #[serde(default = "default_critical_balance")]
    pub critical_balance_usd: f64,
    /// Kept for older config files; unused by the HUD.
    #[serde(default = "default_warn_health")]
    pub warn_health_pct: f64,
    #[serde(default = "default_critical_health")]
    pub critical_health_pct: f64,
    /// Warn when available accounts are at or below this count.
    #[serde(default = "default_warn_available")]
    pub warn_available_count: i64,
    /// Critical when available accounts are at or below this count.
    #[serde(default = "default_critical_available")]
    pub critical_available_count: i64,
    #[serde(default)]
    pub sites: Vec<SiteConfig>,
    /// Last HUD position in logical pixels (so drag survives hide/show).
    #[serde(default)]
    pub panel_x: Option<f64>,
    #[serde(default)]
    pub panel_y: Option<f64>,
}

fn default_refresh() -> u64 {
    60
}

fn default_low_balance() -> f64 {
    1.0
}

fn default_warn_balance() -> f64 {
    5.0
}

fn default_critical_balance() -> f64 {
    1.0
}

fn default_warn_health() -> f64 {
    80.0
}

fn default_critical_health() -> f64 {
    50.0
}

fn default_warn_available() -> i64 {
    5
}

fn default_critical_available() -> i64 {
    2
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            refresh_interval_secs: default_refresh(),
            low_balance_threshold: default_low_balance(),
            warn_balance_usd: default_warn_balance(),
            critical_balance_usd: default_critical_balance(),
            warn_health_pct: default_warn_health(),
            critical_health_pct: default_critical_health(),
            warn_available_count: default_warn_available(),
            critical_available_count: default_critical_available(),
            sites: Vec::new(),
            panel_x: None,
            panel_y: None,
        }
    }
}

/// Public site info returned to the frontend (no secrets).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SitePublic {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub role: SiteRole,
    pub api_key_label: Option<String>,
    pub email: Option<String>,
    pub enabled: bool,
    pub has_api_key: bool,
    pub has_password: bool,
    pub has_token: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitWindow {
    pub window: String,
    pub limit: f64,
    pub used: f64,
    pub remaining: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionUsage {
    pub daily_usage_usd: f64,
    pub weekly_usage_usd: f64,
    pub monthly_usage_usd: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub daily_limit_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekly_limit_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monthly_limit_usd: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub requests: i64,
    pub total_tokens: i64,
    pub cost: f64,
    pub actual_cost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserSnapshot {
    pub site_id: String,
    pub mode: String,
    pub unit: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub balance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_name: Option<String>,
    pub is_valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub today: Option<UsageSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<UsageSummary>,
    #[serde(default)]
    pub rate_limits: Vec<RateLimitWindow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscription: Option<SubscriptionUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpm: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub today_cost: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub month_cost: Option<f64>,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupHealth {
    pub group_id: i64,
    pub group_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    pub total: i64,
    pub available: i64,
    pub rate_limited: i64,
    pub error: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminSnapshot {
    pub site_id: String,
    pub monitoring_enabled: bool,
    /// Status string -> count
    pub status_breakdown: HashMap<String, i64>,
    pub groups: Vec<GroupHealth>,
    pub total_accounts: i64,
    pub available_accounts: i64,
    pub error_accounts: i64,
    pub rate_limited_accounts: i64,
    #[serde(default)]
    pub unschedulable_accounts: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub today_cost: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub month_cost: Option<f64>,
    pub updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteSnapshot {
    pub site: SitePublic,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<UserSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub admin: Option<AdminSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStateView {
    pub settings: AppSettingsPublic,
    pub snapshots: Vec<SiteSnapshot>,
    pub last_refresh_at: Option<String>,
    pub refreshing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettingsPublic {
    pub refresh_interval_secs: u64,
    pub low_balance_threshold: f64,
    pub warn_balance_usd: f64,
    pub critical_balance_usd: f64,
    pub warn_health_pct: f64,
    pub critical_health_pct: f64,
    pub warn_available_count: i64,
    pub critical_available_count: i64,
    pub sites: Vec<SitePublic>,
}

/// Payload for creating/updating a site (secrets optional on update).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SiteUpsert {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    pub base_url: String,
    pub role: SiteRole,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_key_label: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsUpdate {
    #[serde(default)]
    pub refresh_interval_secs: Option<u64>,
    #[serde(default)]
    pub low_balance_threshold: Option<f64>,
    #[serde(default)]
    pub warn_balance_usd: Option<f64>,
    #[serde(default)]
    pub critical_balance_usd: Option<f64>,
    #[serde(default)]
    pub warn_health_pct: Option<f64>,
    #[serde(default)]
    pub critical_health_pct: Option<f64>,
    #[serde(default)]
    pub warn_available_count: Option<i64>,
    #[serde(default)]
    pub critical_available_count: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginResult {
    pub requires_2fa: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temp_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_email_masked: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub message: String,
}
