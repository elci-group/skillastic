//! Skill-use event log: agents record how skills are used, the daemon and
//! `skillastic review` aggregate the events into per-skill statistics.
//!
//! Events are appended as one JSON object per line to
//! `.skillastic/usage.jsonl` — an append-only log, deliberately not part of
//! the registry's atomic rewrite set.

use crate::error::{Result, SkillasticError};
use crate::model::{UsageEvent, UsageKind, UsageStats};
use crate::registry::Registry;
use chrono::Utc;
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

const USAGE_LOG: &str = "usage.jsonl";

/// Parsed usage log: the usable events plus the raw line count, so callers
/// can tell "no events" apart from "no new events" without re-reading.
#[derive(Debug, Default)]
pub struct UsageLog {
    pub events: Vec<UsageEvent>,
    pub lines: u64,
}

pub fn log_path(registry: &Registry) -> PathBuf {
    registry.root().join(USAGE_LOG)
}

/// Append one usage event. The skill must be registered, so typos fail
/// loudly instead of silently feeding orphan events into the log.
pub fn record(
    registry: &Registry,
    skill_name: &str,
    event: UsageKind,
    note: Option<String>,
) -> Result<UsageEvent> {
    if !registry.has_skill(skill_name) {
        return Err(SkillasticError::SkillNotFound(skill_name.to_string()));
    }
    let entry = UsageEvent {
        timestamp: Utc::now(),
        skill: skill_name.to_string(),
        event,
        note,
    };
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path(registry))?;
    writeln!(file, "{}", serde_json::to_string(&entry)?)?;
    Ok(entry)
}

/// Read the whole log. Blank or malformed lines are skipped; `lines` counts
/// the remaining physical lines so review bookkeeping stays stable even when
/// a writer is interrupted mid-line.
pub fn load(registry: &Registry) -> Result<UsageLog> {
    let path = log_path(registry);
    let mut log = UsageLog::default();
    if !path.exists() {
        return Ok(log);
    }
    let raw = std::fs::read_to_string(path)?;
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        log.lines += 1;
        if let Ok(event) = serde_json::from_str::<UsageEvent>(line) {
            log.events.push(event);
        }
    }
    Ok(log)
}

/// Per-skill totals across all recorded events, keyed by skill name.
pub fn aggregate(events: &[UsageEvent]) -> BTreeMap<String, UsageStats> {
    let mut stats: BTreeMap<String, UsageStats> = BTreeMap::new();
    for event in events {
        let entry = stats.entry(event.skill.clone()).or_default();
        match event.event {
            UsageKind::Invoked => entry.invoked += 1,
            UsageKind::Completed => entry.completed += 1,
            UsageKind::Failed => entry.failed += 1,
        }
        if entry.first_seen.is_none() {
            entry.first_seen = Some(event.timestamp);
        }
        entry.last_seen = Some(event.timestamp);
    }
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Config, Skill};
    use semver::Version;
    use tempfile::TempDir;

    fn setup() -> (TempDir, Registry) {
        let dir = TempDir::new().unwrap();
        let registry = Registry::init(dir.path(), Config::default()).unwrap();
        let skill = Skill::new("web", Version::new(1, 0, 0), vec!["*".into()], v("1.0.0"));
        registry.add_skill(&skill, "# Web").unwrap();
        (dir, registry)
    }

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn record_and_load_round_trip() {
        let (_dir, registry) = setup();
        record(&registry, "web", UsageKind::Invoked, None).unwrap();
        record(
            &registry,
            "web",
            UsageKind::Failed,
            Some("wrong flags".into()),
        )
        .unwrap();

        let log = load(&registry).unwrap();
        assert_eq!(log.lines, 2);
        assert_eq!(log.events.len(), 2);
        assert_eq!(log.events[0].skill, "web");
        assert_eq!(log.events[1].event, UsageKind::Failed);
        assert_eq!(log.events[1].note.as_deref(), Some("wrong flags"));
    }

    #[test]
    fn record_rejects_unknown_skill() {
        let (_dir, registry) = setup();
        let err = record(&registry, "nope", UsageKind::Invoked, None).unwrap_err();
        assert!(err.to_string().contains("skill not found: nope"));
        assert!(!log_path(&registry).exists());
    }

    #[test]
    fn load_skips_malformed_lines_but_counts_them() {
        let (_dir, registry) = setup();
        record(&registry, "web", UsageKind::Invoked, None).unwrap();
        let mut raw = std::fs::read_to_string(log_path(&registry)).unwrap();
        raw.push_str("not json\n\n{\"skill\": 1}\n");
        std::fs::write(log_path(&registry), raw).unwrap();

        let log = load(&registry).unwrap();
        assert_eq!(log.lines, 3);
        assert_eq!(log.events.len(), 1);
    }

    #[test]
    fn load_of_missing_log_is_empty() {
        let (_dir, registry) = setup();
        let log = load(&registry).unwrap();
        assert!(log.events.is_empty());
        assert_eq!(log.lines, 0);
    }

    #[test]
    fn aggregate_totals_per_skill() {
        let (_dir, registry) = setup();
        record(&registry, "web", UsageKind::Invoked, None).unwrap();
        record(&registry, "web", UsageKind::Failed, None).unwrap();
        record(&registry, "web", UsageKind::Completed, None).unwrap();

        let log = load(&registry).unwrap();
        let stats = aggregate(&log.events);
        let web = stats.get("web").unwrap();
        assert_eq!((web.invoked, web.completed, web.failed), (1, 1, 1));
        assert!(web.first_seen.is_some());
        assert!(web.last_seen.is_some());
        // failure_rate is failed per invoked use.
        assert!((web.failure_rate() - 1.0).abs() < 1e-9);
    }
}
