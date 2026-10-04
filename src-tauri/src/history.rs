use crate::alerts::{self, AlertState, AlertThresholds, Observation, Severity};
use crate::models::{SiteDelta, SiteSnapshot};
use crate::store::data_dir;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::time::Duration;

const KEEP: Duration = Duration::from_secs(24 * 60 * 60);
const MIN_SPAN: i64 = 60;
const HOUR_MS: i64 = 3_600_000;
const DEDUPE_MS: i64 = 20_000;
const MAX_SAMPLES: usize = 1_500;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HistorySample {
    at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    balance: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    errors: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SiteRecord {
    #[serde(default)]
    samples: Vec<HistorySample>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    alert_severity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    alert_errors: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct HistoryFile {
    #[serde(default)]
    sites: HashMap<String, SiteRecord>,
}

#[derive(Debug, Default)]
pub struct History {
    sites: HashMap<String, SiteRecord>,
}

impl History {
    pub fn load() -> Self {
        let path = data_dir().join("history.json");
        let Ok(raw) = fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str::<HistoryFile>(&raw)
            .map(|file| Self { sites: file.sites })
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let path = data_dir().join("history.json");
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let file = HistoryFile {
            sites: self.sites.clone(),
        };
        if let Ok(raw) = serde_json::to_string(&file) {
            let _ = fs::write(&path, raw);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
            }
        }
    }

    pub fn retain(&mut self, site_ids: &HashSet<String>) {
        self.sites.retain(|id, _| site_ids.contains(id));
    }

    pub fn remove(&mut self, site_id: &str) {
        self.sites.remove(site_id);
    }

    pub fn record_sample(&mut self, snap: &SiteSnapshot, now_ms: i64) {
        let sample = sample_of(snap, now_ms);
        if sample.balance.is_none() && sample.errors.is_none() {
            return;
        }
        let record = self.sites.entry(snap.site.id.clone()).or_default();
        push_sample(&mut record.samples, sample, now_ms);
    }

    /// Returns a notification body when this refresh is worse than the saved stamp.
    pub fn take_alert(
        &mut self,
        snap: &SiteSnapshot,
        thresholds: AlertThresholds,
    ) -> Option<String> {
        let Some(observed) = observe_snapshot(snap, thresholds) else {
            return None;
        };
        let record = self.sites.entry(snap.site.id.clone()).or_default();
        let prev = alert_of(record);
        record.alert_severity = Some(observed.state.severity.as_str().into());
        record.alert_errors = Some(observed.state.error_count);
        alerts::notification(&snap.site.name, prev.as_ref(), &observed)
    }

    pub fn delta_for(&self, snap: &SiteSnapshot, now_ms: i64) -> Option<SiteDelta> {
        let record = self.sites.get(&snap.site.id)?;
        let current = sample_of(snap, now_ms);
        delta_from(&record.samples, now_ms, current.balance, current.errors)
    }
}

fn alert_of(record: &SiteRecord) -> Option<AlertState> {
    Some(AlertState {
        severity: Severity::parse(record.alert_severity.as_deref()?),
        error_count: record.alert_errors.unwrap_or(0),
    })
}

fn observe_snapshot(snap: &SiteSnapshot, thresholds: AlertThresholds) -> Option<Observation> {
    if let Some(admin) = &snap.admin {
        return Some(alerts::observe_admin(admin, thresholds));
    }
    snap.user
        .as_ref()
        .map(|user| alerts::observe_user(user, thresholds))
}

fn sample_of(snap: &SiteSnapshot, now_ms: i64) -> HistorySample {
    let balance = snap.user.as_ref().and_then(|user| {
        user.remaining
            .or(user.balance)
            .filter(|value| value.is_finite())
    });
    let errors = snap
        .admin
        .as_ref()
        .filter(|admin| admin.error.is_none() || admin.total_accounts > 0)
        .map(|admin| admin.error_accounts);
    HistorySample {
        at_ms: now_ms,
        balance,
        errors,
    }
}

fn push_sample(samples: &mut Vec<HistorySample>, sample: HistorySample, now_ms: i64) {
    let cutoff = now_ms - KEEP.as_millis() as i64;
    samples.retain(|item| item.at_ms >= cutoff);
    if let Some(last) = samples.last_mut() {
        if sample.at_ms.saturating_sub(last.at_ms) < DEDUPE_MS {
            *last = sample;
            return;
        }
    }
    samples.push(sample);
    if samples.len() > MAX_SAMPLES {
        let extra = samples.len() - MAX_SAMPLES;
        samples.drain(0..extra);
    }
}

fn delta_from(
    samples: &[HistorySample],
    now_ms: i64,
    balance: Option<f64>,
    errors: Option<i64>,
) -> Option<SiteDelta> {
    if samples.is_empty() {
        return None;
    }
    let hour_ago = now_ms - HOUR_MS;
    let past = samples
        .iter()
        .rev()
        .find(|sample| sample.at_ms <= hour_ago)
        .or_else(|| samples.first())?;
    let span = now_ms.saturating_sub(past.at_ms) / 1000;
    if span < MIN_SPAN {
        return None;
    }
    let balance = match (balance, past.balance) {
        (Some(now), Some(then)) => {
            let delta = now - then;
            (delta.abs() >= 0.005).then_some(delta)
        }
        _ => None,
    };
    let errors = match (errors, past.errors) {
        (Some(now), Some(then)) if now != then => Some(now - then),
        _ => None,
    };
    if balance.is_none() && errors.is_none() {
        return None;
    }
    Some(SiteDelta {
        span_secs: span,
        balance,
        errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(at_ms: i64, balance: Option<f64>, errors: Option<i64>) -> HistorySample {
        HistorySample {
            at_ms,
            balance,
            errors,
        }
    }

    #[test]
    fn delta_uses_the_sample_closest_to_one_hour() {
        let now = 10_000_000_i64;
        let samples = vec![
            sample(now - 2 * HOUR_MS, Some(20.0), Some(0)),
            sample(now - HOUR_MS - 30_000, Some(12.0), Some(1)),
            sample(now - 60_000, Some(11.0), Some(1)),
        ];
        let delta = delta_from(&samples, now, Some(10.8), Some(4)).unwrap();
        assert!((delta.balance.unwrap() - (10.8 - 12.0)).abs() < 0.0001);
        assert_eq!(delta.errors, Some(3));
        assert!(delta.span_secs >= 3600);
    }

    #[test]
    fn short_history_uses_a_shorter_span_and_tiny_changes_hide() {
        let now = 5_000_000_i64;
        let recent = vec![sample(now - 30_000, Some(10.0), Some(1))];
        assert!(delta_from(&recent, now, Some(9.0), Some(2)).is_none());

        let quarter = vec![sample(now - 15 * 60 * 1000, Some(10.0), Some(2))];
        let delta = delta_from(&quarter, now, Some(10.0), Some(2));
        assert!(delta.is_none());
        let moved = delta_from(&quarter, now, Some(8.5), Some(2)).unwrap();
        assert_eq!(moved.span_secs, 15 * 60);
        assert!(moved.errors.is_none());
        assert!((moved.balance.unwrap() - (-1.5)).abs() < 0.0001);
    }

    #[test]
    fn samples_older_than_a_day_are_dropped() {
        let now = 50_000_000_i64;
        let mut samples = vec![sample(now - KEEP.as_millis() as i64 - 1, Some(1.0), None)];
        push_sample(&mut samples, sample(now, Some(2.0), None), now);
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].balance, Some(2.0));
    }
}
