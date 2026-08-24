use crate::models::{
    AdminSnapshot, GroupHealth, RateLimitWindow, SubscriptionUsage, UsageSummary, UserSnapshot,
};
use crate::store::Store;
use chrono::{Datelike, Local, Utc};
use serde_json::Value;
use std::collections::HashMap;
use thiserror::Error;

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
}

impl Sub2Client {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .user_agent(format!("sub2viewer/{}", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("http client");
        Self { http }
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
            return Err(ClientError::Message(format!("Token 刷新失败 (HTTP {status})")));
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
        let url = Self::join(base_url, "/v1/usage?days=31");
        match self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {api_key}"))
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
        let mut snap = match self.fetch_account_availability(base_url, &token).await {
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
                            snap.error = Some(format!(
                                "实时监控未开启，账号列表回退失败: {e}"
                            ));
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
                        self.attach_admin_usage(base_url, &token, &mut snap).await;
                        return snap;
                    }
                    if let Ok(mut snap) = self.fetch_accounts_fallback(base_url, &token).await {
                        snap.site_id = site_id.to_string();
                        self.attach_admin_usage(base_url, &token, &mut snap).await;
                        return snap;
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
        self.attach_admin_usage(base_url, &token, &mut snap).await;
        snap
    }

    async fn attach_admin_usage(&self, base_url: &str, token: &str, snap: &mut AdminSnapshot) {
        for path in [
            "/api/v1/admin/dashboard/stats",
            "/api/v1/admin/dashboard/snapshot-v2",
        ] {
            let url = Self::join(base_url, path);
            let Ok(resp) = self
                .http
                .get(&url)
                .header("Authorization", format!("Bearer {token}"))
                .send()
                .await
            else {
                continue;
            };
            if !resp.status().is_success() {
                continue;
            }
            let Ok(body) = resp.json::<Value>().await else {
                continue;
            };
            let data = body.get("data").cloned().unwrap_or(body);
            if let Some(today) = pick_cost(&data, &["today_cost", "todayCost", "today_spend"])
                .or_else(|| data.pointer("/today/cost").and_then(|v| v.as_f64()))
                .or_else(|| data.pointer("/today/actual_cost").and_then(|v| v.as_f64()))
            {
                snap.today_cost = Some(today);
            }
            if let Some(month) = pick_cost(
                &data,
                &[
                    "month_cost",
                    "monthCost",
                    "monthly_cost",
                    "monthly_usage_usd",
                    "month_spend",
                ],
            )
            .or_else(|| data.pointer("/month/cost").and_then(|v| v.as_f64()))
            .or_else(|| data.pointer("/monthly/cost").and_then(|v| v.as_f64()))
            {
                snap.month_cost = Some(month);
            }
            if snap.today_cost.is_some() || snap.month_cost.is_some() {
                break;
            }
        }
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
            return Err(ClientError::Message(format!(
                "Unauthorized ({status})"
            )));
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
                    total: v.get("total_accounts").and_then(|x| x.as_i64()).unwrap_or(0),
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
            if v.get("is_available").and_then(|x| x.as_bool()).unwrap_or(false) {
                available += 1;
            }
            if v.get("has_error").and_then(|x| x.as_bool()).unwrap_or(false) {
                errors += 1;
            }
            if v
                .get("is_rate_limited")
                .and_then(|x| x.as_bool())
                .unwrap_or(false)
            {
                rate_limited += 1;
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
            .or_else(|| {
                body.get("data")
                    .and_then(|d| d.as_array())
                    .cloned()
            })
            .unwrap_or_default();

        let mut status_breakdown: HashMap<String, i64> = HashMap::new();
        let mut group_map: HashMap<i64, GroupHealth> = HashMap::new();
        let mut available = 0i64;
        let mut errors = 0i64;
        let mut rate_limited = 0i64;

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

            if is_error {
                errors += 1;
            } else if is_rate {
                rate_limited += 1;
            } else if is_active && schedulable {
                available += 1;
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
            today_cost: None,
            month_cost: None,
            updated_at: now_iso(),
            error: None,
        })
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
        today_cost: None,
        month_cost: None,
        updated_at: now_iso(),
        error: Some(msg),
    }
}

fn parse_usage_body(site_id: &str, body: Value) -> UserSnapshot {
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
    let balance = body.get("balance").and_then(|v| v.as_f64());
    let remaining = body
        .get("remaining")
        .and_then(|v| v.as_f64())
        .or_else(|| {
            body.pointer("/quota/remaining")
                .and_then(|v| v.as_f64())
        });
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
                limit: item.get("limit").and_then(|v| v.as_f64()).unwrap_or(0.0),
                used: item.get("used").and_then(|v| v.as_f64()).unwrap_or(0.0),
                remaining: item
                    .get("remaining")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
                reset_at: item
                    .get("reset_at")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
            });
        }
    }

    let subscription = body.get("subscription").map(|s| SubscriptionUsage {
        daily_usage_usd: s
            .get("daily_usage_usd")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        weekly_usage_usd: s
            .get("weekly_usage_usd")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        monthly_usage_usd: s
            .get("monthly_usage_usd")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        daily_limit_usd: s.get("daily_limit_usd").and_then(|v| v.as_f64()),
        weekly_limit_usd: s.get("weekly_limit_usd").and_then(|v| v.as_f64()),
        monthly_limit_usd: s.get("monthly_limit_usd").and_then(|v| v.as_f64()),
        expires_at: s
            .get("expires_at")
            .and_then(|v| v.as_str())
            .map(|x| x.to_string()),
    });

    let today = body.pointer("/usage/today").map(parse_usage_summary);
    let total = body.pointer("/usage/total").map(parse_usage_summary);
    let rpm = body.pointer("/usage/rpm").and_then(|v| v.as_f64());
    let (series_today, series_month) = sum_daily_usage(&body);

    let today_cost = today
        .as_ref()
        .map(|t| if t.actual_cost > 0.0 { t.actual_cost } else { t.cost })
        .or_else(|| subscription.as_ref().map(|s| s.daily_usage_usd))
        .or(series_today);
    let month_cost = subscription
        .as_ref()
        .map(|s| s.monthly_usage_usd)
        .filter(|v| *v > 0.0)
        .or(series_month);

    // quota_limited also may expose quota
    let remaining = remaining.or_else(|| {
        body.pointer("/quota/remaining")
            .and_then(|v| v.as_f64())
    });
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

fn pick_cost(v: &Value, keys: &[&str]) -> Option<f64> {
    for k in keys {
        if let Some(n) = v.get(*k).and_then(|x| x.as_f64()) {
            return Some(n);
        }
    }
    None
}

fn value_cost(item: &Value) -> f64 {
    item.get("actual_cost")
        .and_then(|v| v.as_f64())
        .or_else(|| item.get("cost").and_then(|v| v.as_f64()))
        .or_else(|| item.get("total_cost").and_then(|v| v.as_f64()))
        .or_else(|| item.get("spend").and_then(|v| v.as_f64()))
        .unwrap_or(0.0)
}

fn item_date(item: &Value) -> Option<String> {
    item.get("date")
        .or_else(|| item.get("day"))
        .or_else(|| item.get("time"))
        .and_then(|v| v.as_str())
        .map(|s| s.chars().take(10).collect())
}

fn sum_daily_usage(body: &Value) -> (Option<f64>, Option<f64>) {
    let arr = body
        .get("daily_usage")
        .or_else(|| body.get("dailyUsage"))
        .and_then(|v| v.as_array());
    let Some(arr) = arr else {
        return (None, None);
    };
    if arr.is_empty() {
        return (None, None);
    }
    let now = Local::now().date_naive();
    let today = now.format("%Y-%m-%d").to_string();
    let month_prefix = format!("{:04}-{:02}", now.year(), now.month());
    let mut today_sum = 0.0;
    let mut month_sum = 0.0;
    let mut saw = false;
    for item in arr {
        let cost = value_cost(item);
        let Some(date) = item_date(item) else {
            continue;
        };
        saw = true;
        if date == today {
            today_sum += cost;
        }
        if date.starts_with(&month_prefix) {
            month_sum += cost;
        }
    }
    if !saw {
        return (None, None);
    }
    (Some(today_sum), Some(month_sum))
}

fn parse_usage_summary(v: &Value) -> UsageSummary {
    UsageSummary {
        requests: v.get("requests").and_then(|x| x.as_i64()).unwrap_or(0),
        total_tokens: v
            .get("total_tokens")
            .and_then(|x| x.as_i64())
            .unwrap_or(0),
        cost: v.get("cost").and_then(|x| x.as_f64()).unwrap_or(0.0),
        actual_cost: v
            .get("actual_cost")
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0),
    }
}


