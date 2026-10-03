use crate::models::{
    AdminSnapshot, ApiKeyBinding, BindableGroup, GroupHealth, RateLimitWindow, SubscriptionUsage,
    UsageSummary, UserSnapshot,
};
use crate::store::Store;
use chrono::{DateTime, Datelike, Local, NaiveDate, Utc};
use reqwest::header::{HeaderMap, HeaderValue, CACHE_CONTROL, PRAGMA};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use thiserror::Error;
use url::form_urlencoded;

const KEY_PAGE_SIZE: i64 = 100;
const MAX_API_KEYS: usize = 200;
const MAX_BINDABLE_GROUPS: usize = 50;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("{0}")]
    Message(String),
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Clone)]
pub struct Sub2Client {
    http: reqwest::Client,
    /// base url -> whether PUT /admin/api-keys/:id exists. Probed once per process.
    switch_support: Arc<Mutex<HashMap<String, bool>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyCatalog {
    pub keys: Vec<ApiKeyBinding>,
    pub groups: Vec<BindableGroup>,
    pub truncated: bool,
    pub list_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySwitchError {
    Failed(String),
    Unauthorized,
    Unsupported,
}

impl Sub2Client {
    pub fn new() -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(
            CACHE_CONTROL,
            HeaderValue::from_static("no-cache, no-store, max-age=0"),
        );
        headers.insert(PRAGMA, HeaderValue::from_static("no-cache"));
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .user_agent(format!("sub2viewer/{}", env!("CARGO_PKG_VERSION")))
            .default_headers(headers)
            .build()
            .expect("http client");
        Self {
            http,
            switch_support: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn join(base: &str, path: &str) -> String {
        let base = base.trim_end_matches('/');
        let path = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };
        format!("{base}{path}")
    }

    fn join_query(base: &str, path: &str, pairs: &[(&str, &str)]) -> String {
        let mut ser = form_urlencoded::Serializer::new(String::new());
        for (k, v) in pairs {
            ser.append_pair(k, v);
        }
        // Cache-bust GET URLs so reverse proxies cannot freeze usage numbers.
        ser.append_pair("_ts", &Utc::now().timestamp_millis().to_string());
        format!("{}?{}", Self::join(base, path), ser.finish())
    }

    /// Login with email/password. Returns (access, refresh, role) or 2FA challenge.
    pub async fn login(
        &self,
        base_url: &str,
        email: &str,
        password: &str,
    ) -> Result<LoginOutcome, ClientError> {
        let url = Self::join(base_url, "/api/v1/auth/login");
        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({
                "email": email,
                "password": password,
            }))
            .send()
            .await?;

        let status = resp.status();
        let body: Value = resp.json().await?;
        if !status.is_success() {
            let msg = body
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("登录失败");
            return Err(ClientError::Message(format!("{msg} (HTTP {status})")));
        }

        let code = body.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
        if code != 0 {
            let msg = body
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("登录失败");
            return Err(ClientError::Message(msg.to_string()));
        }

        let data = body.get("data").cloned().unwrap_or(Value::Null);
        if data
            .get("requires_2fa")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Ok(LoginOutcome::Requires2FA {
                temp_token: data
                    .get("temp_token")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                user_email_masked: data
                    .get("user_email_masked")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            });
        }

        let access = data
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ClientError::Message("登录响应缺少 access_token".into()))?
            .to_string();
        let refresh = data
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let role = data
            .pointer("/user/role")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Ok(LoginOutcome::Success {
            access_token: access,
            refresh_token: refresh,
            role,
        })
    }

    pub async fn login_2fa(
        &self,
        base_url: &str,
        temp_token: &str,
        totp_code: &str,
    ) -> Result<LoginOutcome, ClientError> {
        let url = Self::join(base_url, "/api/v1/auth/login/2fa");
        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({
                "temp_token": temp_token,
                "totp_code": totp_code,
            }))
            .send()
            .await?;
        let status = resp.status();
        let body: Value = resp.json().await?;
        if !status.is_success() {
            let msg = body
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("2FA 验证失败");
            return Err(ClientError::Message(format!("{msg} (HTTP {status})")));
        }
        let data = body.get("data").cloned().unwrap_or(Value::Null);
        let access = data
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ClientError::Message("2FA 响应缺少 access_token".into()))?
            .to_string();
        let refresh = data
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let role = data
            .pointer("/user/role")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        Ok(LoginOutcome::Success {
            access_token: access,
            refresh_token: refresh,
            role,
        })
    }

    pub async fn refresh_token(
        &self,
        base_url: &str,
        refresh_token: &str,
    ) -> Result<(String, Option<String>), ClientError> {
        let url = Self::join(base_url, "/api/v1/auth/refresh");
        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({ "refresh_token": refresh_token }))
            .send()
            .await?;
        let status = resp.status();
        let body: Value = resp.json().await?;
        if !status.is_success() {
            return Err(ClientError::Message(format!(
                "Token 刷新失败 (HTTP {status})"
            )));
        }
        let data = body.get("data").cloned().unwrap_or(Value::Null);
        let access = data
            .get("access_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ClientError::Message("刷新响应缺少 access_token".into()))?
            .to_string();
        let refresh = data
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        Ok((access, refresh))
    }

    pub async fn fetch_user_usage(
        &self,
        base_url: &str,
        api_key: &str,
        site_id: &str,
    ) -> UserSnapshot {
        let tz = local_timezone();
        let days = usage_lookback_days().to_string();
        let url = Self::join_query(
            base_url,
            "/v1/usage",
            &[("days", days.as_str()), ("timezone", tz.as_str())],
        );
        match self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Cache-Control", "no-cache")
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                match resp.json::<Value>().await {
                    Ok(body) => {
                        if !status.is_success() {
                            let msg = body
                                .get("error")
                                .and_then(|e| e.get("message"))
                                .and_then(|v| v.as_str())
                                .or_else(|| body.get("message").and_then(|v| v.as_str()))
                                .unwrap_or("用量查询失败");
                            return UserSnapshot {
                                site_id: site_id.to_string(),
                                mode: "error".into(),
                                unit: "USD".into(),
                                balance: None,
                                remaining: None,
                                plan_name: None,
                                is_valid: false,
                                status: Some(format!("http_{status}")),
                                today: None,
                                total: None,
                                rate_limits: vec![],
                                subscription: None,
                                rpm: None,
                                today_cost: None,
                                month_cost: None,
                                updated_at: now_iso(),
                                error: Some(msg.to_string()),
                            };
                        }
                        parse_usage_body(site_id, body)
                    }
                    Err(e) => error_user(site_id, e.to_string()),
                }
            }
            Err(e) => error_user(site_id, e.to_string()),
        }
    }

    async fn ensure_access_token(
        &self,
        site_id: &str,
        base_url: &str,
        email: Option<&str>,
        password: Option<&str>,
    ) -> Result<String, ClientError> {
        if let Some(token) = Store::get_secret(site_id, "access_token") {
            return Ok(token);
        }
        if let Some(refresh) = Store::get_secret(site_id, "refresh_token") {
            match self.refresh_token(base_url, &refresh).await {
                Ok((access, new_refresh)) => {
                    let _ = Store::set_secret(site_id, "access_token", &access);
                    if let Some(r) = new_refresh {
                        let _ = Store::set_secret(site_id, "refresh_token", &r);
                    }
                    return Ok(access);
                }
                Err(_) => {
                    Store::delete_secret(site_id, "access_token");
                    Store::delete_secret(site_id, "refresh_token");
                }
            }
        }
        let email = email
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ClientError::Message("缺少管理员邮箱".into()))?;
        let password_owned = password
            .map(|s| s.to_string())
            .or_else(|| Store::get_secret(site_id, "password"))
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ClientError::Message("缺少管理员密码，请先登录".into()))?;
        let password = password_owned.as_str();

        match self.login(base_url, email, password).await? {
            LoginOutcome::Success {
                access_token,
                refresh_token,
                role,
            } => {
                if let Some(r) = role.as_deref() {
                    if r != "admin" && r != "Admin" {
                        // still allow if server returns other privileged roles
                        if r != "super_admin" {
                            // continue — admin APIs will fail clearly if unauthorized
                        }
                    }
                }
                let _ = Store::set_secret(site_id, "access_token", &access_token);
                if let Some(r) = refresh_token {
                    let _ = Store::set_secret(site_id, "refresh_token", &r);
                }
                Ok(access_token)
            }
            LoginOutcome::Requires2FA { .. } => Err(ClientError::Message(
                "该账号启用了 2FA，请在设置中完成二次验证登录".into(),
            )),
        }
    }

    pub async fn fetch_admin(
        &self,
        site_id: &str,
        base_url: &str,
        email: Option<&str>,
    ) -> AdminSnapshot {
        let password = Store::get_secret(site_id, "password");
        let token = match self
            .ensure_access_token(site_id, base_url, email, password.as_deref())
            .await
        {
            Ok(t) => t,
            Err(e) => return error_admin(site_id, e.to_string()),
        };

        // Prefer realtime availability; fall back to accounts list aggregation.
        let snap = match self.fetch_account_availability(base_url, &token).await {
            Ok(mut snap) => {
                snap.site_id = site_id.to_string();
                if snap.error.is_none() && !snap.monitoring_enabled {
                    match self.fetch_accounts_fallback(base_url, &token).await {
                        Ok(mut fb) => {
                            fb.site_id = site_id.to_string();
                            fb.monitoring_enabled = false;
                            fb
                        }
                        Err(e) => {
                            snap.error = Some(format!("实时监控未开启，账号列表回退失败: {e}"));
                            snap.updated_at = now_iso();
                            snap
                        }
                    }
                } else {
                    snap
                }
            }
            Err(e) => {
                if e.to_string().contains("401") || e.to_string().contains("Unauthorized") {
                    Store::delete_secret(site_id, "access_token");
                    let token = match self
                        .ensure_access_token(site_id, base_url, email, password.as_deref())
                        .await
                    {
                        Ok(t) => t,
                        Err(e2) => return error_admin(site_id, e2.to_string()),
                    };
                    if let Ok(mut snap) = self.fetch_account_availability(base_url, &token).await {
                        snap.site_id = site_id.to_string();
                        return self
                            .finish_admin(site_id, base_url, email, &token, snap)
                            .await;
                    }
                    if let Ok(mut snap) = self.fetch_accounts_fallback(base_url, &token).await {
                        snap.site_id = site_id.to_string();
                        return self
                            .finish_admin(site_id, base_url, email, &token, snap)
                            .await;
                    }
                }
                match self.fetch_accounts_fallback(base_url, &token).await {
                    Ok(mut snap) => {
                        snap.site_id = site_id.to_string();
                        snap
                    }
                    Err(e2) => return error_admin(site_id, format!("{e}; fallback: {e2}")),
                }
            }
        };
        self.finish_admin(site_id, base_url, email, &token, snap)
            .await
    }

    async fn finish_admin(
        &self,
        site_id: &str,
        base_url: &str,
        email: Option<&str>,
        token: &str,
        mut snap: AdminSnapshot,
    ) -> AdminSnapshot {
        self.attach_admin_usage(base_url, token, &mut snap).await;
        self.attach_key_catalog(site_id, base_url, email, token, &mut snap)
            .await;
        snap
    }

    async fn attach_admin_usage(&self, base_url: &str, token: &str, snap: &mut AdminSnapshot) {
        let tz = local_timezone();
        let today = Local::now().date_naive();
        let month_start = today.with_day(1).unwrap_or(today);
        let (today_cost, month_cost) = tokio::join!(
            self.fetch_admin_period_cost(base_url, token, today, today, &tz),
            self.fetch_admin_period_cost(base_url, token, month_start, today, &tz),
        );
        if let Some(v) = today_cost {
            snap.today_cost = Some(v);
        }
        if let Some(v) = month_cost {
            snap.month_cost = Some(v);
        }
        if snap.today_cost.is_some() && snap.month_cost.is_some() {
            return;
        }
        self.attach_admin_usage_legacy(base_url, token, snap).await;
    }

    async fn fetch_admin_period_cost(
        &self,
        base_url: &str,
        token: &str,
        start: NaiveDate,
        end: NaiveDate,
        tz: &str,
    ) -> Option<f64> {
        let start_s = start.format("%Y-%m-%d").to_string();
        let end_s = end.format("%Y-%m-%d").to_string();
        let url = Self::join_query(
            base_url,
            "/api/v1/admin/usage/stats",
            &[
                ("start_date", start_s.as_str()),
                ("end_date", end_s.as_str()),
                ("timezone", tz),
                ("nocache", "true"),
            ],
        );
        let body = self.get_json(&url, token).await?;
        extract_actual_cost(&unwrap_payload(&body))
    }

    async fn attach_admin_usage_legacy(
        &self,
        base_url: &str,
        token: &str,
        snap: &mut AdminSnapshot,
    ) {
        if snap.today_cost.is_none() {
            let url = Self::join_query(base_url, "/api/v1/admin/dashboard/stats", &[]);
            if let Some(body) = self.get_json(&url, token).await {
                let data = unwrap_payload(&body);
                if let Some(today) = extract_today_spend(&data) {
                    snap.today_cost = Some(today);
                }
            }
        }

        if snap.today_cost.is_some() && snap.month_cost.is_some() {
            return;
        }

        let tz = local_timezone();
        let today = Local::now().date_naive();
        let month_start = today.with_day(1).unwrap_or(today);
        let start_s = month_start.format("%Y-%m-%d").to_string();
        let end_s = today.format("%Y-%m-%d").to_string();
        let url = Self::join_query(
            base_url,
            "/api/v1/admin/dashboard/snapshot-v2",
            &[
                ("start_date", start_s.as_str()),
                ("end_date", end_s.as_str()),
                ("timezone", tz.as_str()),
                ("granularity", "day"),
                ("include_stats", "true"),
                ("include_trend", "true"),
                ("include_model_stats", "false"),
                ("include_group_stats", "false"),
            ],
        );
        let Some(body) = self.get_json(&url, token).await else {
            return;
        };
        let data = unwrap_payload(&body);
        if snap.today_cost.is_none() {
            if let Some(today) = extract_today_spend(&data)
                .or_else(|| data.get("stats").and_then(extract_today_spend))
            {
                snap.today_cost = Some(today);
            }
        }
        if snap.month_cost.is_none() {
            let today_s = today.format("%Y-%m-%d").to_string();
            let month_prefix = format!("{:04}-{:02}", today.year(), today.month());
            if let Some(month) = sum_daily_usage_on(&data, &today_s, &month_prefix).1 {
                snap.month_cost = Some(month);
            }
        }
    }

    async fn get_json(&self, url: &str, bearer: &str) -> Option<Value> {
        let resp = self
            .http
            .get(url)
            .header("Authorization", format!("Bearer {bearer}"))
            .header("Cache-Control", "no-cache")
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        resp.json().await.ok()
    }

    async fn fetch_account_availability(
        &self,
        base_url: &str,
        token: &str,
    ) -> Result<AdminSnapshot, ClientError> {
        let url = Self::join(base_url, "/api/v1/admin/ops/account-availability");
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await?;
        let status = resp.status();
        let body: Value = resp.json().await?;
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err(ClientError::Message(format!("Unauthorized ({status})")));
        }
        if !status.is_success() {
            let msg = body
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("account-availability 请求失败");
            return Err(ClientError::Message(format!("{msg} ({status})")));
        }
        let data = body.get("data").cloned().unwrap_or(Value::Null);
        let enabled = data
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let mut groups: Vec<GroupHealth> = Vec::new();
        if let Some(group_map) = data.get("group").and_then(|v| v.as_object()) {
            for (_k, v) in group_map {
                groups.push(GroupHealth {
                    group_id: v.get("group_id").and_then(|x| x.as_i64()).unwrap_or(0),
                    group_name: v
                        .get("group_name")
                        .and_then(|x| x.as_str())
                        .unwrap_or("unknown")
                        .to_string(),
                    platform: v
                        .get("platform")
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_string()),
                    total: v
                        .get("total_accounts")
                        .and_then(|x| x.as_i64())
                        .unwrap_or(0),
                    available: v
                        .get("available_count")
                        .and_then(|x| x.as_i64())
                        .unwrap_or(0),
                    rate_limited: v
                        .get("rate_limit_count")
                        .and_then(|x| x.as_i64())
                        .unwrap_or(0),
                    error: v.get("error_count").and_then(|x| x.as_i64()).unwrap_or(0),
                });
            }
        }

        // Never sum group totals: one account in two groups would be counted twice
        // (often +1). Unique account_id is the source of truth.
        let mut total = 0i64;
        let mut available = 0i64;
        let mut errors = 0i64;
        let mut rate_limited = 0i64;
        let mut unschedulable = 0i64;
        let mut status_breakdown: HashMap<String, i64> = HashMap::new();
        let mut seen: HashMap<i64, ()> = HashMap::new();

        let account_map = data.get("account").and_then(|v| v.as_object());
        if account_map.is_none() || account_map.is_some_and(|m| m.is_empty()) {
            return Err(ClientError::Message(
                "availability has no unique account list".into(),
            ));
        }

        for (k, v) in account_map.unwrap() {
            if !v.is_object() {
                continue;
            }
            let id = v
                .get("account_id")
                .and_then(|x| x.as_i64())
                .or_else(|| k.parse::<i64>().ok())
                .unwrap_or(0);
            if id <= 0 || seen.contains_key(&id) {
                continue;
            }
            seen.insert(id, ());
            total += 1;
            let status = v
                .get("status")
                .and_then(|x| x.as_str())
                .unwrap_or("unknown")
                .to_string();
            *status_breakdown.entry(status).or_insert(0) += 1;
            match classify_ops_account(v) {
                AccountClass::Available => available += 1,
                AccountClass::Error => errors += 1,
                AccountClass::RateLimited => rate_limited += 1,
                AccountClass::Unschedulable => unschedulable += 1,
            }
        }

        groups.sort_by(|a, b| a.group_name.cmp(&b.group_name));

        Ok(AdminSnapshot {
            site_id: String::new(),
            monitoring_enabled: enabled,
            status_breakdown,
            groups,
            total_accounts: total,
            available_accounts: available,
            error_accounts: errors,
            rate_limited_accounts: rate_limited,
            unschedulable_accounts: unschedulable,
            api_keys: Vec::new(),
            bindable_groups: Vec::new(),
            key_switch_supported: false,
            keys_truncated: false,
            key_list_error: None,
            today_cost: None,
            month_cost: None,
            updated_at: now_iso(),
            error: None,
        })
    }

    async fn fetch_accounts_fallback(
        &self,
        base_url: &str,
        token: &str,
    ) -> Result<AdminSnapshot, ClientError> {
        let url = Self::join(
            base_url,
            "/api/v1/admin/accounts?page=1&page_size=200&lite=true&sort_by=name&sort_order=asc",
        );
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await?;
        let status = resp.status();
        let body: Value = resp.json().await?;
        if !status.is_success() {
            let msg = body
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("账号列表请求失败");
            return Err(ClientError::Message(format!("{msg} ({status})")));
        }
        let items = body
            .pointer("/data/items")
            .and_then(|v| v.as_array())
            .cloned()
            .or_else(|| body.get("data").and_then(|d| d.as_array()).cloned())
            .unwrap_or_default();

        let mut status_breakdown: HashMap<String, i64> = HashMap::new();
        let mut group_map: HashMap<i64, GroupHealth> = HashMap::new();
        let mut available = 0i64;
        let mut errors = 0i64;
        let mut rate_limited = 0i64;
        let mut unschedulable = 0i64;

        for acc in &items {
            let id = acc
                .get("id")
                .or_else(|| acc.get("account_id"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            if id <= 0 {
                continue;
            }
            let acc_type = acc
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if acc_type.contains("shadow") {
                continue;
            }
            if acc.get("deleted_at").map(|v| !v.is_null()).unwrap_or(false) {
                continue;
            }

            let status = acc
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            *status_breakdown.entry(status.clone()).or_insert(0) += 1;

            let schedulable = acc
                .get("schedulable")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let is_error = status == "error";
            let is_active = status == "active";
            let is_rate = acc
                .get("rate_limit_reset_at")
                .and_then(|v| v.as_str())
                .is_some();

            let class = if is_error {
                AccountClass::Error
            } else if is_rate {
                AccountClass::RateLimited
            } else if is_active && schedulable {
                AccountClass::Available
            } else {
                AccountClass::Unschedulable
            };
            match class {
                AccountClass::Available => available += 1,
                AccountClass::Error => errors += 1,
                AccountClass::RateLimited => rate_limited += 1,
                AccountClass::Unschedulable => unschedulable += 1,
            }

            // groups can be array or nested
            let groups = acc
                .get("groups")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if groups.is_empty() {
                let entry = group_map.entry(0).or_insert_with(|| GroupHealth {
                    group_id: 0,
                    group_name: "未分组".into(),
                    platform: acc
                        .get("platform")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string()),
                    total: 0,
                    available: 0,
                    rate_limited: 0,
                    error: 0,
                });
                entry.total += 1;
                if is_error {
                    entry.error += 1;
                } else if is_rate {
                    entry.rate_limited += 1;
                } else if is_active && schedulable {
                    entry.available += 1;
                }
            } else {
                for g in groups {
                    let gid = g.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
                    let gname = g
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let entry = group_map.entry(gid).or_insert_with(|| GroupHealth {
                        group_id: gid,
                        group_name: gname.clone(),
                        platform: g
                            .get("platform")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string()),
                        total: 0,
                        available: 0,
                        rate_limited: 0,
                        error: 0,
                    });
                    entry.total += 1;
                    if is_error {
                        entry.error += 1;
                    } else if is_rate {
                        entry.rate_limited += 1;
                    } else if is_active && schedulable {
                        entry.available += 1;
                    }
                }
            }
        }

        let mut groups: Vec<GroupHealth> = group_map.into_values().collect();
        groups.sort_by(|a, b| a.group_name.cmp(&b.group_name));
        let total = status_breakdown.values().sum::<i64>();

        Ok(AdminSnapshot {
            site_id: String::new(),
            monitoring_enabled: false,
            status_breakdown,
            groups,
            total_accounts: total,
            available_accounts: available,
            error_accounts: errors,
            rate_limited_accounts: rate_limited,
            unschedulable_accounts: unschedulable,
            api_keys: Vec::new(),
            bindable_groups: Vec::new(),
            key_switch_supported: false,
            keys_truncated: false,
            key_list_error: None,
            today_cost: None,
            month_cost: None,
            updated_at: now_iso(),
            error: None,
        })
    }

    /// Change one API key's group. `group_id` 0 clears the binding.
    /// Does not send `reset_rate_limit_usage`.
    pub async fn switch_api_key_group(
        &self,
        site_id: &str,
        base_url: &str,
        email: Option<&str>,
        key_id: i64,
        group_id: i64,
    ) -> Result<(), KeySwitchError> {
        let password = Store::get_secret(site_id, "password");
        let token = self
            .ensure_access_token(site_id, base_url, email, password.as_deref())
            .await
            .map_err(|e| KeySwitchError::Failed(e.to_string()))?;
        match self.put_key_group(base_url, &token, key_id, group_id).await {
            Err(KeySwitchError::Unauthorized) => {
                Store::delete_secret(site_id, "access_token");
                let token = self
                    .ensure_access_token(site_id, base_url, email, password.as_deref())
                    .await
                    .map_err(|e| KeySwitchError::Failed(e.to_string()))?;
                self.put_key_group(base_url, &token, key_id, group_id).await
            }
            other => other,
        }
    }

    pub fn remember_switch_support(&self, base_url: &str, supported: bool) {
        let mut map = self
            .switch_support
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        map.insert(base_url.trim_end_matches('/').to_string(), supported);
    }

    pub async fn load_key_state(
        &self,
        site_id: &str,
        base_url: &str,
        email: Option<&str>,
    ) -> Result<(KeyCatalog, bool), ClientError> {
        let password = Store::get_secret(site_id, "password");
        let token = self
            .ensure_access_token(site_id, base_url, email, password.as_deref())
            .await?;
        match self.load_key_state_with_token(base_url, &token).await {
            Err(e) if is_unauthorized(&e) => {
                Store::delete_secret(site_id, "access_token");
                let token = self
                    .ensure_access_token(site_id, base_url, email, password.as_deref())
                    .await?;
                self.load_key_state_with_token(base_url, &token).await
            }
            other => other,
        }
    }

    async fn attach_key_catalog(
        &self,
        site_id: &str,
        base_url: &str,
        email: Option<&str>,
        token: &str,
        snap: &mut AdminSnapshot,
    ) {
        let loaded = match self.load_key_state_with_token(base_url, token).await {
            Ok(v) => Ok(v),
            Err(e) if is_unauthorized(&e) => {
                Store::delete_secret(site_id, "access_token");
                match self
                    .ensure_access_token(
                        site_id,
                        base_url,
                        email,
                        Store::get_secret(site_id, "password").as_deref(),
                    )
                    .await
                {
                    Ok(token) => self.load_key_state_with_token(base_url, &token).await,
                    Err(e2) => Err(e2),
                }
            }
            Err(e) => Err(e),
        };
        match loaded {
            Ok((cat, supported)) => apply_catalog(snap, cat, supported),
            Err(e) => {
                snap.api_keys.clear();
                snap.bindable_groups.clear();
                snap.keys_truncated = false;
                snap.key_list_error = Some(e.to_string());
            }
        }
    }

    async fn load_key_state_with_token(
        &self,
        base_url: &str,
        token: &str,
    ) -> Result<(KeyCatalog, bool), ClientError> {
        let (catalog, support) = tokio::join!(
            self.fetch_key_catalog(base_url, token),
            self.probe_switch_support(base_url, token),
        );
        let catalog = catalog?;
        let supported = match support {
            Ok(v) => v,
            Err(e) if is_unauthorized(&e) => return Err(e),
            Err(_) => true,
        };
        Ok((catalog, supported))
    }

    async fn probe_switch_support(&self, base_url: &str, token: &str) -> Result<bool, ClientError> {
        let key = base_url.trim_end_matches('/').to_string();
        if let Some(known) = self
            .switch_support
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&key)
            .copied()
        {
            return Ok(known);
        }
        // Non-numeric id: a deployed route answers 400, a missing route answers 404.
        // This does not change any key.
        let url = Self::join(base_url, "/api/v1/admin/api-keys/abc");
        let resp = self
            .http
            .put(&url)
            .header("Authorization", format!("Bearer {token}"))
            .json(&serde_json::json!({ "group_id": 1 }))
            .send()
            .await?;
        let status = resp.status();
        let _ = resp.bytes().await;
        if status.as_u16() == 401 {
            return Err(ClientError::Message("401 Unauthorized".into()));
        }
        let supported = status.as_u16() != 404;
        self.remember_switch_support(base_url, supported);
        Ok(supported)
    }

    async fn put_key_group(
        &self,
        base_url: &str,
        token: &str,
        key_id: i64,
        group_id: i64,
    ) -> Result<(), KeySwitchError> {
        let url = Self::join(base_url, &format!("/api/v1/admin/api-keys/{key_id}"));
        let resp = self
            .http
            .put(&url)
            .header("Authorization", format!("Bearer {token}"))
            .json(&serde_json::json!({ "group_id": group_id }))
            .send()
            .await
            .map_err(|e| KeySwitchError::Failed(e.to_string()))?;
        let status = resp.status();
        if status.is_success() {
            let _ = resp.bytes().await;
            return Ok(());
        }
        if status.as_u16() == 401 {
            let _ = resp.bytes().await;
            return Err(KeySwitchError::Unauthorized);
        }
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        let msg = body
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if status.as_u16() == 404
            && (msg.is_empty()
                || msg.eq_ignore_ascii_case("404")
                || msg.to_ascii_lowercase().contains("page not found"))
        {
            self.remember_switch_support(base_url, false);
            return Err(KeySwitchError::Unsupported);
        }
        let msg = if msg.is_empty() {
            "切换分组失败".to_string()
        } else {
            truncate_chars(msg, 120)
        };
        Err(KeySwitchError::Failed(msg))
    }

    async fn fetch_key_catalog(
        &self,
        base_url: &str,
        token: &str,
    ) -> Result<KeyCatalog, ClientError> {
        let mut list_error: Option<String> = None;
        let groups = match self
            .admin_get(base_url, token, "/api/v1/admin/groups/all", &[])
            .await
        {
            Ok(body) => {
                let mut groups = parse_bindable_groups(&body);
                if groups.len() > MAX_BINDABLE_GROUPS {
                    groups.truncate(MAX_BINDABLE_GROUPS);
                }
                groups
            }
            Err(e) if is_unauthorized(&e) => return Err(e),
            Err(e) => {
                list_error = Some(format!("分组列表失败: {e}"));
                Vec::new()
            }
        };

        let mut keys = Vec::new();
        let mut reported = 0i64;
        let mut truncated = false;
        let mut page = 1i64;
        loop {
            let page_s = page.to_string();
            let size_s = KEY_PAGE_SIZE.to_string();
            let body = match self
                .admin_get(
                    base_url,
                    token,
                    "/api/v1/admin/users",
                    &[("page", page_s.as_str()), ("page_size", size_s.as_str())],
                )
                .await
            {
                Ok(body) => body,
                Err(e) if is_unauthorized(&e) => return Err(e),
                Err(e) => {
                    list_error = Some(format!("用户列表失败: {e}"));
                    break;
                }
            };
            let (ids, total_users) = parse_id_page(&body);
            if ids.is_empty() {
                break;
            }
            for user_id in ids {
                if keys.len() >= MAX_API_KEYS {
                    truncated = true;
                    break;
                }
                match self
                    .fetch_user_keys(base_url, token, user_id, MAX_API_KEYS - keys.len())
                    .await
                {
                    Ok((batch, total)) => {
                        reported += total;
                        let batch_len = batch.len() as i64;
                        if batch_len < total {
                            truncated = true;
                        }
                        keys.extend(batch);
                    }
                    Err(e) if is_unauthorized(&e) => return Err(e),
                    Err(e) => {
                        list_error = Some(format!("密钥列表失败: {e}"));
                        truncated = true;
                        break;
                    }
                }
            }
            if truncated || keys.len() >= MAX_API_KEYS {
                if (page * KEY_PAGE_SIZE) < total_users {
                    truncated = true;
                }
                break;
            }
            if page * KEY_PAGE_SIZE >= total_users || page >= 20 {
                break;
            }
            page += 1;
        }
        if reported > keys.len() as i64 {
            truncated = true;
        }
        resolve_key_group_names(&mut keys, &groups);
        keys.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.id.cmp(&b.id))
        });

        Ok(KeyCatalog {
            keys,
            groups,
            truncated,
            list_error,
        })
    }

    async fn fetch_user_keys(
        &self,
        base_url: &str,
        token: &str,
        user_id: i64,
        remaining: usize,
    ) -> Result<(Vec<ApiKeyBinding>, i64), ClientError> {
        let mut out = Vec::new();
        let mut total = 0i64;
        let mut page = 1i64;
        let path = format!("/api/v1/admin/users/{user_id}/api-keys");
        loop {
            let page_s = page.to_string();
            let size_s = KEY_PAGE_SIZE.to_string();
            let body = self
                .admin_get(
                    base_url,
                    token,
                    &path,
                    &[("page", page_s.as_str()), ("page_size", size_s.as_str())],
                )
                .await?;
            let (batch, page_total) = parse_api_key_page(&body);
            total = page_total.max(total);
            if batch.is_empty() {
                break;
            }
            for key in batch {
                if out.len() >= remaining {
                    break;
                }
                out.push(key);
            }
            if out.len() >= remaining || out.len() as i64 >= total || page >= 10 {
                break;
            }
            page += 1;
        }
        Ok((out, total))
    }

    async fn admin_get(
        &self,
        base_url: &str,
        token: &str,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<Value, ClientError> {
        let url = Self::join_query(base_url, path, query);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        if status.as_u16() == 401 {
            return Err(ClientError::Message("401 Unauthorized".into()));
        }
        if !status.is_success() {
            let msg = body
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("请求失败");
            return Err(ClientError::Message(format!("{msg} ({status})")));
        }
        Ok(body)
    }
}

fn apply_catalog(snap: &mut AdminSnapshot, cat: KeyCatalog, supported: bool) {
    snap.api_keys = cat.keys;
    snap.bindable_groups = cat.groups;
    snap.keys_truncated = cat.truncated;
    snap.key_list_error = cat.list_error;
    snap.key_switch_supported = supported;
}

fn is_unauthorized(err: &ClientError) -> bool {
    let text = err.to_string();
    text.contains("401") || text.contains("Unauthorized")
}

fn truncate_chars(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn parse_bindable_groups(body: &Value) -> Vec<BindableGroup> {
    let data = unwrap_payload(body);
    let items = data
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .or_else(|| data.as_array().cloned())
        .unwrap_or_default();
    let mut raw = Vec::new();
    for g in items {
        let id = g.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
        if id <= 0 {
            continue;
        }
        let status = g.get("status").and_then(|v| v.as_str()).unwrap_or("active");
        if status != "active" {
            continue;
        }
        let name = g
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if name.is_empty() {
            continue;
        }
        let sort = g.get("sort_order").and_then(|v| v.as_i64()).unwrap_or(0);
        let platform = g
            .get("platform")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        raw.push((
            sort,
            name.to_lowercase(),
            BindableGroup { id, name, platform },
        ));
    }
    raw.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.id.cmp(&b.2.id)));
    raw.into_iter().map(|(_, _, g)| g).collect()
}

fn parse_id_page(body: &Value) -> (Vec<i64>, i64) {
    let data = unwrap_payload(body);
    let items = data
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let total = data
        .get("total")
        .and_then(|v| v.as_i64())
        .unwrap_or(items.len() as i64);
    let ids = items
        .iter()
        .filter_map(|u| u.get("id").and_then(|v| v.as_i64()))
        .filter(|id| *id > 0)
        .collect();
    (ids, total)
}

fn parse_api_key_page(body: &Value) -> (Vec<ApiKeyBinding>, i64) {
    let data = unwrap_payload(body);
    let items = data
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .or_else(|| data.as_array().cloned())
        .unwrap_or_default();
    let total = data
        .get("total")
        .and_then(|v| v.as_i64())
        .unwrap_or(items.len() as i64);
    let keys = items.iter().filter_map(parse_api_key_item).collect();
    (keys, total)
}

fn parse_api_key_item(v: &Value) -> Option<ApiKeyBinding> {
    let id = v.get("id").and_then(|x| x.as_i64()).filter(|id| *id > 0)?;
    let mut name = v
        .get("name")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if name.is_empty() {
        name = format!("密钥 #{id}");
    }
    let group_id = v
        .get("group_id")
        .and_then(|x| x.as_i64())
        .filter(|id| *id > 0)
        .or_else(|| {
            v.pointer("/group/id")
                .and_then(|x| x.as_i64())
                .filter(|id| *id > 0)
        });
    let group_name = v
        .pointer("/group/name")
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let status = v
        .get("status")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("active")
        .to_string();
    Some(ApiKeyBinding {
        id,
        name,
        group_id,
        group_name,
        status,
        switch_error: None,
    })
}

fn resolve_key_group_names(keys: &mut [ApiKeyBinding], groups: &[BindableGroup]) {
    for key in keys.iter_mut() {
        let missing = key
            .group_name
            .as_ref()
            .map(|s| s.is_empty())
            .unwrap_or(true);
        if !missing {
            continue;
        }
        if let Some(id) = key.group_id {
            key.group_name = groups.iter().find(|g| g.id == id).map(|g| g.name.clone());
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum AccountClass {
    Available,
    Error,
    RateLimited,
    Unschedulable,
}

fn classify_ops_account(v: &Value) -> AccountClass {
    let has_error = v
        .get("has_error")
        .and_then(|x| x.as_bool())
        .unwrap_or(false)
        || v.get("status").and_then(|x| x.as_str()) == Some("error");
    let rate_limited = v
        .get("is_rate_limited")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let available = v
        .get("is_available")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    if has_error {
        AccountClass::Error
    } else if rate_limited {
        AccountClass::RateLimited
    } else if available {
        AccountClass::Available
    } else {
        AccountClass::Unschedulable
    }
}

pub enum LoginOutcome {
    Success {
        access_token: String,
        refresh_token: Option<String>,
        role: Option<String>,
    },
    Requires2FA {
        temp_token: String,
        user_email_masked: Option<String>,
    },
}

fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

fn error_user(site_id: &str, msg: String) -> UserSnapshot {
    UserSnapshot {
        site_id: site_id.to_string(),
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
        updated_at: now_iso(),
        error: Some(msg),
    }
}

fn error_admin(site_id: &str, msg: String) -> AdminSnapshot {
    AdminSnapshot {
        site_id: site_id.to_string(),
        monitoring_enabled: false,
        status_breakdown: HashMap::new(),
        groups: vec![],
        total_accounts: 0,
        available_accounts: 0,
        error_accounts: 0,
        rate_limited_accounts: 0,
        unschedulable_accounts: 0,
        api_keys: Vec::new(),
        bindable_groups: Vec::new(),
        key_switch_supported: false,
        keys_truncated: false,
        key_list_error: None,
        today_cost: None,
        month_cost: None,
        updated_at: now_iso(),
        error: Some(msg),
    }
}

fn parse_usage_body(site_id: &str, body: Value) -> UserSnapshot {
    let body = unwrap_payload(&body);
    let mode = body
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let unit = body
        .get("unit")
        .and_then(|v| v.as_str())
        .unwrap_or("USD")
        .to_string();
    let is_valid = body
        .get("isValid")
        .or_else(|| body.get("is_valid"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let status = body
        .get("status")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let balance = body.get("balance").and_then(json_f64);
    let remaining = body
        .get("remaining")
        .and_then(json_f64)
        .or_else(|| body.pointer("/quota/remaining").and_then(json_f64));
    let plan_name = body
        .get("planName")
        .or_else(|| body.get("plan_name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mut rate_limits = Vec::new();
    if let Some(arr) = body.get("rate_limits").and_then(|v| v.as_array()) {
        for item in arr {
            rate_limits.push(RateLimitWindow {
                window: item
                    .get("window")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?")
                    .to_string(),
                limit: item.get("limit").and_then(json_f64).unwrap_or(0.0),
                used: item.get("used").and_then(json_f64).unwrap_or(0.0),
                remaining: item.get("remaining").and_then(json_f64).unwrap_or(0.0),
                reset_at: item
                    .get("reset_at")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            });
        }
    }

    let subscription = body.get("subscription").map(|s| SubscriptionUsage {
        daily_usage_usd: s.get("daily_usage_usd").and_then(json_f64).unwrap_or(0.0),
        weekly_usage_usd: s.get("weekly_usage_usd").and_then(json_f64).unwrap_or(0.0),
        monthly_usage_usd: s.get("monthly_usage_usd").and_then(json_f64).unwrap_or(0.0),
        daily_limit_usd: s.get("daily_limit_usd").and_then(json_f64),
        weekly_limit_usd: s.get("weekly_limit_usd").and_then(json_f64),
        monthly_limit_usd: s.get("monthly_limit_usd").and_then(json_f64),
        expires_at: s
            .get("expires_at")
            .and_then(|v| v.as_str())
            .map(|x| x.to_string()),
    });

    let today = body.pointer("/usage/today").map(parse_usage_summary);
    let total = body.pointer("/usage/total").map(parse_usage_summary);
    let rpm = body.pointer("/usage/rpm").and_then(json_f64);
    let (series_today, series_month) = sum_daily_usage(&body);
    // /v1/usage usage.today.actual_cost is the same "今日实际扣除" as the
    // Sub2API Keys / Dashboard cards. daily_usage date buckets can disagree
    // around timezone midnight, so they are only used for the month rollup.
    let dash_today = today.as_ref().map(|t| t.actual_cost);
    let today_cost = dash_today.or(series_today);
    let month_cost = match (series_month, series_today, today_cost) {
        (Some(month), Some(series_day), Some(live)) => Some((month - series_day + live).max(0.0)),
        (Some(month), None, Some(live)) => Some(month + live),
        (Some(month), _, None) => Some(month),
        (None, _, live) => live,
    };

    let remaining = remaining.or_else(|| body.pointer("/quota/remaining").and_then(json_f64));
    let balance = balance.or(remaining);

    UserSnapshot {
        site_id: site_id.to_string(),
        mode,
        unit,
        balance,
        remaining,
        plan_name,
        is_valid,
        status,
        today,
        total,
        rate_limits,
        subscription,
        rpm,
        today_cost,
        month_cost,
        updated_at: now_iso(),
        error: None,
    }
}

fn json_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn pick_cost(v: &Value, keys: &[&str]) -> Option<f64> {
    for k in keys {
        if let Some(n) = v.get(*k).and_then(json_f64) {
            if n.is_finite() {
                return Some(n);
            }
        }
    }
    None
}

fn extract_actual_cost(v: &Value) -> Option<f64> {
    pick_cost(
        v,
        &[
            "total_actual_cost",
            "totalActualCost",
            "today_actual_cost",
            "todayActualCost",
            "actual_cost",
            "actualCost",
        ],
    )
    .or_else(|| {
        pick_cost(
            v,
            &["total_cost", "totalCost", "today_cost", "todayCost", "cost"],
        )
    })
}

fn extract_today_spend(v: &Value) -> Option<f64> {
    pick_cost(
        v,
        &[
            "today_actual_cost",
            "todayActualCost",
            "today_cost",
            "todayCost",
            "today_spend",
        ],
    )
    .or_else(|| v.pointer("/today/actual_cost").and_then(json_f64))
    .or_else(|| v.pointer("/today/cost").and_then(json_f64))
    .or_else(|| v.pointer("/stats/today_actual_cost").and_then(json_f64))
    .or_else(|| v.pointer("/stats/today_cost").and_then(json_f64))
}

fn value_cost(item: &Value) -> f64 {
    pick_cost(
        item,
        &[
            "actual_cost",
            "actualCost",
            "cost",
            "total_actual_cost",
            "totalActualCost",
            "total_cost",
            "totalCost",
            "spend",
        ],
    )
    .unwrap_or(0.0)
}

fn item_date(item: &Value) -> Option<String> {
    let v = item
        .get("date")
        .or_else(|| item.get("day"))
        .or_else(|| item.get("time"))
        .or_else(|| item.get("bucket_date"))
        .or_else(|| item.get("bucketDate"))?;
    if let Some(s) = v.as_str() {
        let s = s.trim();
        if s.len() >= 10 && s.as_bytes().get(4) == Some(&b'-') && s.as_bytes().get(7) == Some(&b'-')
        {
            return Some(s.chars().take(10).collect());
        }
        if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
            return Some(dt.with_timezone(&Local).format("%Y-%m-%d").to_string());
        }
    }
    let n = v
        .as_i64()
        .or_else(|| v.as_u64().map(|u| u as i64))
        .or_else(|| json_f64(v).map(|f| f as i64))?;
    let secs = if n > 10_000_000_000 { n / 1000 } else { n };
    DateTime::from_timestamp(secs, 0)
        .map(|dt| dt.with_timezone(&Local).format("%Y-%m-%d").to_string())
}

fn daily_usage_items(body: &Value) -> Option<Vec<Value>> {
    const PATHS: &[&str] = &[
        "/daily_usage",
        "/dailyUsage",
        "/usage/daily_usage",
        "/usage/daily",
        "/trend",
    ];
    for path in PATHS {
        let Some(node) = body.pointer(path) else {
            continue;
        };
        if let Some(arr) = node.as_array() {
            return Some(arr.clone());
        }
        if let Some(obj) = node.as_object() {
            if let Some(arr) = obj
                .get("items")
                .or_else(|| obj.get("data"))
                .or_else(|| obj.get("list"))
                .and_then(|v| v.as_array())
            {
                return Some(arr.clone());
            }
        }
    }
    None
}

fn sum_daily_usage(body: &Value) -> (Option<f64>, Option<f64>) {
    let now = Local::now().date_naive();
    let today = now.format("%Y-%m-%d").to_string();
    let month_prefix = format!("{:04}-{:02}", now.year(), now.month());
    sum_daily_usage_on(body, &today, &month_prefix)
}

fn sum_daily_usage_on(body: &Value, today: &str, month_prefix: &str) -> (Option<f64>, Option<f64>) {
    let Some(arr) = daily_usage_items(body) else {
        return (None, None);
    };
    if arr.is_empty() {
        return (None, None);
    }
    let mut today_sum = 0.0;
    let mut month_sum = 0.0;
    let mut saw = false;
    let mut saw_today = false;
    for item in &arr {
        let Some(date) = item_date(item) else {
            continue;
        };
        saw = true;
        let cost = value_cost(item);
        if date == today {
            saw_today = true;
            today_sum += cost;
        }
        if date.starts_with(month_prefix) {
            month_sum += cost;
        }
    }
    if !saw {
        return (None, None);
    }
    (
        if saw_today { Some(today_sum) } else { None },
        Some(month_sum),
    )
}

fn parse_usage_summary(v: &Value) -> UsageSummary {
    UsageSummary {
        requests: v
            .get("requests")
            .and_then(|x| x.as_i64().or_else(|| json_f64(x).map(|n| n as i64)))
            .unwrap_or(0),
        total_tokens: v
            .get("total_tokens")
            .or_else(|| v.get("totalTokens"))
            .and_then(|x| x.as_i64().or_else(|| json_f64(x).map(|n| n as i64)))
            .unwrap_or(0),
        cost: v.get("cost").and_then(json_f64).unwrap_or(0.0),
        actual_cost: v
            .get("actual_cost")
            .or_else(|| v.get("actualCost"))
            .and_then(json_f64)
            .unwrap_or(0.0),
    }
}

fn unwrap_payload(body: &Value) -> Value {
    let Some(map) = body.as_object() else {
        return body.clone();
    };
    if !(map.contains_key("code") || map.contains_key("success") || map.contains_key("message")) {
        return body.clone();
    }
    match map.get("data") {
        Some(data) if !data.is_null() => data.clone(),
        _ => body.clone(),
    }
}

fn usage_lookback_days() -> u32 {
    Local::now().date_naive().day().max(32).min(90)
}

fn local_timezone() -> String {
    if let Ok(tz) = std::env::var("TZ") {
        let tz = tz.trim().trim_start_matches(':');
        if !tz.is_empty() && !tz.eq_ignore_ascii_case("localtime") {
            return tz.to_string();
        }
    }
    if let Some(name) = timezone_from_localtime_link() {
        return name;
    }
    offset_etc_gmt()
}

fn timezone_from_localtime_link() -> Option<String> {
    for candidate in ["/etc/localtime", "/var/db/timezone/localtime"] {
        let Ok(target) = std::fs::read_link(candidate) else {
            continue;
        };
        let s = target.to_string_lossy().replace('\\', "/");
        for marker in ["zoneinfo/", "TimeZone/"] {
            if let Some(i) = s.rfind(marker) {
                let name = s[i + marker.len()..].trim_matches('/');
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

fn offset_etc_gmt() -> String {
    let hours = Local::now().offset().local_minus_utc() / 3600;
    if hours == 0 {
        "UTC".into()
    } else {
        // Etc/GMT uses inverted signs: GMT-8 is UTC+8.
        format!("Etc/GMT{:+}", -hours)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ymd(days_ago: i64) -> String {
        (Local::now().date_naive() - chrono::Duration::days(days_ago))
            .format("%Y-%m-%d")
            .to_string()
    }

    #[test]
    fn live_today_actual_not_replaced_by_series_or_subscription() {
        let today = ymd(0);
        // Stay inside the current calendar month. Three days ago is last month on the 1st–3rd.
        let earlier = Local::now()
            .date_naive()
            .with_day(1)
            .unwrap()
            .format("%Y-%m-%d")
            .to_string();
        let body = json!({
            "mode": "unrestricted",
            "balance": 12.5,
            "subscription": {
                "daily_usage_usd": 99.0,
                "monthly_usage_usd": 999.0
            },
            "usage": {
                "today": { "requests": 0, "total_tokens": 0, "cost": 1.0, "actual_cost": 0.0 }
            },
            "daily_usage": [
                { "date": earlier, "cost": 2.0, "actual_cost": 1.5 },
                { "date": today, "cost": 4.0, "actual_cost": 3.25 }
            ]
        });
        let snap = parse_usage_body("s1", body);
        assert_eq!(snap.today_cost, Some(0.0));
        assert_eq!(snap.month_cost, Some(1.5));
    }

    #[test]
    fn dashboard_today_used_when_series_missing() {
        let body = json!({
            "usage": {
                "today": {
                    "requests": 4,
                    "total_tokens": 100,
                    "cost": 8.0,
                    "actual_cost": 6.5
                }
            }
        });
        let snap = parse_usage_body("s1", body);
        assert_eq!(snap.today_cost, Some(6.5));
        assert_eq!(snap.month_cost, Some(6.5));
    }

    #[test]
    fn daily_usage_items_wrapper_and_actual_cost() {
        let today = ymd(0);
        let body = json!({
            "daily_usage": {
                "items": [
                    { "date": today, "cost": "5.00", "actual_cost": "4.20" }
                ]
            }
        });
        let (today_cost, month_cost) = sum_daily_usage(&body);
        assert_eq!(today_cost, Some(4.20));
        assert_eq!(month_cost, Some(4.20));
    }

    #[test]
    fn unwraps_admin_envelope_actual_cost() {
        let body = json!({
            "code": 0,
            "message": "ok",
            "data": { "total_actual_cost": 11.11, "total_cost": 20.0 }
        });
        assert_eq!(extract_actual_cost(&unwrap_payload(&body)), Some(11.11));
    }

    #[test]
    fn empty_daily_usage_falls_back_to_dashboard_today() {
        let body = json!({
            "daily_usage": [],
            "usage": {
                "today": { "requests": 2, "total_tokens": 8, "cost": 3.0, "actual_cost": 2.5 }
            }
        });
        let snap = parse_usage_body("s1", body);
        assert_eq!(snap.today_cost, Some(2.5));
        assert_eq!(snap.month_cost, Some(2.5));
    }

    #[test]
    fn missing_today_row_falls_back_to_dashboard() {
        let earlier = ymd(2);
        let body = json!({
            "usage": {
                "today": { "requests": 3, "total_tokens": 9, "cost": 7.0, "actual_cost": 6.0 }
            },
            "daily_usage": [
                { "date": earlier, "actual_cost": 1.0 }
            ]
        });
        let snap = parse_usage_body("s1", body);
        assert_eq!(snap.today_cost, Some(6.0));
        assert_eq!(snap.month_cost, Some(7.0));
    }

    #[test]
    fn month_sum_ignores_previous_month() {
        let today = Local::now().date_naive();
        let this_month = today.with_day(1).unwrap().format("%Y-%m-%d").to_string();
        let prev = (today.with_day(1).unwrap() - chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let body = json!({
            "daily_usage": [
                { "date": prev, "actual_cost": 50.0 },
                { "date": this_month, "actual_cost": 1.0 }
            ]
        });
        let month_prefix = format!("{:04}-{:02}", today.year(), today.month());
        let today_s = today.format("%Y-%m-%d").to_string();
        let (_t, month) = sum_daily_usage_on(&body, &today_s, &month_prefix);
        assert_eq!(month, Some(1.0));
    }

    #[test]
    fn classify_ops_account_partitions_status() {
        assert_eq!(
            classify_ops_account(
                &json!({"is_available": true, "has_error": false, "is_rate_limited": false})
            ),
            AccountClass::Available
        );
        assert_eq!(
            classify_ops_account(
                &json!({"is_available": false, "has_error": true, "is_rate_limited": true})
            ),
            AccountClass::Error
        );
        assert_eq!(
            classify_ops_account(
                &json!({"is_available": false, "has_error": false, "is_rate_limited": true})
            ),
            AccountClass::RateLimited
        );
        assert_eq!(
            classify_ops_account(
                &json!({"is_available": false, "has_error": false, "is_rate_limited": false, "is_overloaded": true})
            ),
            AccountClass::Unschedulable
        );
    }

    #[test]
    fn parse_api_keys_drops_secret_and_keeps_group() {
        let body = json!({
            "code": 0,
            "message": "success",
            "data": {
                "items": [{
                    "id": 7,
                    "name": "Gpt-free",
                    "key": "sk-secret-value",
                    "group_id": 2,
                    "status": "active",
                    "user": { "email": "person@example.com", "id": 3 },
                    "group": { "id": 2, "name": "ChatGpt(free号池)" },
                    "ip_whitelist": ["10.0.0.1"],
                    "last_used_ip": "10.1.1.1"
                }],
                "total": 1
            }
        });
        let (keys, total) = parse_api_key_page(&body);
        assert_eq!(total, 1);
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].name, "Gpt-free");
        assert_eq!(keys[0].group_id, Some(2));
        assert_eq!(keys[0].group_name.as_deref(), Some("ChatGpt(free号池)"));
        let dumped = serde_json::to_string(&keys[0]).unwrap();
        assert!(!dumped.contains("sk-secret"));
        assert!(!dumped.contains("person@example.com"));
        assert!(!dumped.contains("10.0.0.1"));
        assert!(!dumped.contains("10.1.1.1"));
    }

    #[test]
    fn unbound_key_has_no_group_and_empty_name_gets_fallback() {
        let body = json!({
            "code": 0,
            "message": "success",
            "data": {
                "items": [{ "id": 4, "name": "  ", "group_id": 0, "status": "active" }],
                "total": 1
            }
        });
        let (keys, _) = parse_api_key_page(&body);
        assert_eq!(keys[0].name, "密钥 #4");
        assert_eq!(keys[0].group_id, None);
        assert_eq!(keys[0].group_name, None);
    }

    #[test]
    fn parse_groups_keeps_active_and_sorts() {
        let body = json!({
            "code": 0,
            "data": [
                { "id": 5, "name": "上游中转", "status": "active", "platform": "openai", "sort_order": 4 },
                { "id": 9, "name": "停用池", "status": "disabled", "sort_order": 0 },
                { "id": 2, "name": "ChatGpt(free号池)", "status": "active", "platform": "openai", "sort_order": 1 },
                { "id": 1, "name": "", "status": "active", "sort_order": 0 }
            ]
        });
        let groups = parse_bindable_groups(&body);
        assert_eq!(groups.iter().map(|g| g.id).collect::<Vec<_>>(), vec![2, 5]);
        assert_eq!(groups[0].platform.as_deref(), Some("openai"));
    }

    #[test]
    fn resolve_group_name_from_catalog_when_nested_group_missing() {
        let mut keys = vec![ApiKeyBinding {
            id: 3,
            name: "Gpt-pro".into(),
            group_id: Some(3),
            group_name: None,
            status: "active".into(),
            switch_error: None,
        }];
        let groups = vec![BindableGroup {
            id: 3,
            name: "ChatGpt(team号池)".into(),
            platform: None,
        }];
        resolve_key_group_names(&mut keys, &groups);
        assert_eq!(keys[0].group_name.as_deref(), Some("ChatGpt(team号池)"));
    }
}
