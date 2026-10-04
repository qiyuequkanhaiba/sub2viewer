use crate::models::{AdminSnapshot, UserSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Ok = 0,
    Warn = 1,
    Bad = 2,
    Down = 3,
    Auth = 4,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warn => "warn",
            Self::Bad => "bad",
            Self::Down => "down",
            Self::Auth => "auth",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw {
            "warn" => Self::Warn,
            "bad" => Self::Bad,
            "down" => Self::Down,
            "auth" => Self::Auth,
            _ => Self::Ok,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertState {
    pub severity: Severity,
    pub error_count: i64,
}

#[derive(Debug, Clone)]
pub struct Observation {
    pub state: AlertState,
    pub detail: String,
}

#[derive(Debug, Clone, Copy)]
pub struct AlertThresholds {
    pub warn_balance: f64,
    pub critical_balance: f64,
    pub warn_available: i64,
    pub critical_available: i64,
}

pub fn observe_user(user: &UserSnapshot, thresholds: AlertThresholds) -> Observation {
    let balance = user.remaining.or(user.balance);
    if let Some(err) = user.error.as_deref() {
        if balance.is_none() {
            return Observation {
                state: AlertState {
                    severity: Severity::Down,
                    error_count: 0,
                },
                detail: scrub_notice(err).unwrap_or_else(|| "无法读取用量".into()),
            };
        }
    }
    let (severity, detail) = match balance {
        Some(value) if value < 0.0 => (Severity::Ok, "余额不限".into()),
        Some(value) if value <= thresholds.critical_balance => {
            (Severity::Bad, format!("余额 {}", money(value)))
        }
        Some(value) if value <= thresholds.warn_balance => {
            (Severity::Warn, format!("余额 {}", money(value)))
        }
        Some(value) => (Severity::Ok, format!("余额 {}", money(value))),
        None => (Severity::Ok, String::new()),
    };
    Observation {
        state: AlertState {
            severity,
            error_count: 0,
        },
        detail,
    }
}

pub fn observe_admin(admin: &AdminSnapshot, thresholds: AlertThresholds) -> Observation {
    let failed = admin.error.as_deref().filter(|_| admin.total_accounts <= 0);
    if let Some(err) = failed {
        let severity = if is_auth_failure(err) {
            Severity::Auth
        } else {
            Severity::Down
        };
        return Observation {
            state: AlertState {
                severity,
                error_count: admin.error_accounts,
            },
            detail: scrub_notice(err).unwrap_or_else(|| "无法读取健康度".into()),
        };
    }
    let available = admin.available_accounts;
    let severity = if available <= thresholds.critical_available {
        Severity::Bad
    } else if available <= thresholds.warn_available {
        Severity::Warn
    } else {
        Severity::Ok
    };
    Observation {
        state: AlertState {
            severity,
            error_count: admin.error_accounts,
        },
        detail: format!("可用账号 {available}"),
    }
}

/// Notify only when the site gets worse than the last saved state.
/// The first observation is silent so an upgrade does not repeat the current status.
pub fn notification(site: &str, prev: Option<&AlertState>, next: &Observation) -> Option<String> {
    let prev = prev?;
    let worse = next.state.severity > prev.severity;
    let errors_up = next.state.error_count > prev.error_count;
    if !worse && !errors_up {
        return None;
    }
    let mut body = format!("{site} {}", phrase(next.state.severity, worse));
    if !next.detail.is_empty() && worse {
        body.push('：');
        body.push_str(&next.detail);
    }
    if errors_up {
        let delta = next.state.error_count - prev.error_count;
        if worse {
            body.push_str(&format!("，错误 +{delta}"));
        } else {
            body = format!("{site} 错误账号 +{delta}，当前 {}", next.state.error_count);
        }
    }
    Some(truncate_chars(&body, 140))
}

fn phrase(severity: Severity, worse: bool) -> &'static str {
    if !worse {
        return "";
    }
    match severity {
        Severity::Warn => "进入预警",
        Severity::Bad => "进入告警",
        Severity::Down => "请求失败",
        Severity::Auth => "登录已过期",
        Severity::Ok => "已恢复",
    }
}

fn is_auth_failure(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("401")
        || lower.contains("unauthorized")
        || msg.contains("登录")
        || msg.contains("2FA")
        || msg.contains("密码")
}

fn money(value: f64) -> String {
    if value < 0.0 {
        return "∞".into();
    }
    if value >= 100.0 || (value - value.round()).abs() < 0.001 {
        format!("${:.0}", value)
    } else {
        format!("${value:.2}")
    }
}

pub(crate) fn scrub_notice(raw: &str) -> Option<String> {
    let mut cleaned = String::new();
    for token in raw.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if lower.starts_with("sk-")
            || lower.starts_with("bearer")
            || token.contains('@')
            || token.contains("://")
        {
            continue;
        }
        let token = token.trim_matches(|c: char| matches!(c, '"' | '\'' | ',' | ';' | '。'));
        if token.is_empty() {
            continue;
        }
        if !cleaned.is_empty() {
            cleaned.push(' ');
        }
        cleaned.push_str(token);
    }
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        None
    } else {
        Some(truncate_chars(cleaned, 80))
    }
}

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

    fn thresholds() -> AlertThresholds {
        AlertThresholds {
            warn_balance: 5.0,
            critical_balance: 1.0,
            warn_available: 5,
            critical_available: 2,
        }
    }

    fn state(severity: Severity, error_count: i64) -> AlertState {
        AlertState {
            severity,
            error_count,
        }
    }

    fn obs(severity: Severity, error_count: i64, detail: &str) -> Observation {
        Observation {
            state: state(severity, error_count),
            detail: detail.into(),
        }
    }

    #[test]
    fn first_sight_is_silent_and_same_state_stays_silent() {
        let next = obs(Severity::Bad, 2, "可用账号 1");
        assert!(notification("Quyue", None, &next).is_none());
        let prev = state(Severity::Bad, 2);
        assert!(notification("Quyue", Some(&prev), &next).is_none());
    }

    #[test]
    fn worsening_notifies_once_and_recovery_rearms() {
        let prev = state(Severity::Ok, 0);
        let warn = obs(Severity::Warn, 0, "余额 $4.00");
        let body = notification("个人", Some(&prev), &warn).unwrap();
        assert!(body.contains("进入预警"));
        assert!(body.contains("$4.00"));

        let still = obs(Severity::Warn, 0, "余额 $3.00");
        assert!(notification("个人", Some(&warn.state), &still).is_none());

        let bad = obs(Severity::Bad, 0, "余额 $0.40");
        assert!(notification("个人", Some(&warn.state), &bad)
            .unwrap()
            .contains("进入告警"));

        let recovered = obs(Severity::Ok, 0, "余额 $20");
        assert!(notification("个人", Some(&bad.state), &recovered).is_none());
        assert!(notification("个人", Some(&recovered.state), &bad).is_some());
    }

    #[test]
    fn error_increase_notifies_without_repeating_the_same_count() {
        let prev = state(Severity::Bad, 1);
        let next = obs(Severity::Bad, 4, "可用账号 1");
        let body = notification("Quyue", Some(&prev), &next).unwrap();
        assert!(body.contains("错误账号 +3"));
        assert!(body.contains("当前 4"));
        assert!(notification("Quyue", Some(&next.state), &next).is_none());
    }

    #[test]
    fn auth_failure_outranks_a_plain_outage() {
        let admin = AdminSnapshot {
            site_id: "s".into(),
            monitoring_enabled: false,
            status_breakdown: Default::default(),
            groups: vec![],
            total_accounts: 0,
            available_accounts: 0,
            error_accounts: 0,
            rate_limited_accounts: 0,
            unschedulable_accounts: 0,
            issues: vec![],
            api_keys: vec![],
            bindable_groups: vec![],
            key_switch_supported: false,
            keys_truncated: false,
            key_list_error: None,
            today_cost: None,
            month_cost: None,
            updated_at: String::new(),
            error: Some("Unauthorized (401)".into()),
        };
        let seen = observe_admin(&admin, thresholds());
        assert_eq!(seen.state.severity, Severity::Auth);
        let body = notification("Quyue", Some(&state(Severity::Ok, 0)), &seen).unwrap();
        assert!(body.contains("登录已过期"));
    }

    #[test]
    fn scrub_drops_secrets_and_addresses() {
        let cleaned =
            scrub_notice("upstream sk-secret-value user@example.com https://x.test failed")
                .unwrap();
        assert!(!cleaned.contains("sk-"));
        assert!(!cleaned.contains('@'));
        assert!(!cleaned.contains("http"));
        assert!(cleaned.contains("upstream"));
        assert!(cleaned.contains("failed"));
    }
}
