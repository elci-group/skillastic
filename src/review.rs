//! Deterministic skill-use review: aggregates the usage log against the
//! registered skills and flags the ones that need attention. Pure local
//! computation — inference only enters later, in `suggest`.

use crate::error::Result;
use crate::model::{
    DaemonState, Finding, FindingKind, ReviewReport, ReviewStatus, Severity, SkillReview,
};
use crate::registry::Registry;
use crate::usage::{self, UsageLog};
use chrono::Utc;
use semver::Version;

/// Minimum invocations before a failure rate is meaningful.
const HIGH_FAILURE_MIN_INVOCATIONS: u64 = 3;
/// Failure rate at or above which a skill is flagged.
const HIGH_FAILURE_RATE: f64 = 0.5;

pub fn run(registry: &Registry, app_version: &Version) -> Result<ReviewReport> {
    let skills = registry.list_skills()?;
    let log = usage::load(registry)?;
    let stats = usage::aggregate(&log.events);

    // Events may reference skills that were removed after being recorded.
    let known: std::collections::BTreeSet<&str> = skills.iter().map(|s| s.name.as_str()).collect();
    let orphan_events = stats
        .iter()
        .filter(|(name, _)| !known.contains(name.as_str()))
        .map(|(_, s)| s.invoked + s.completed + s.failed)
        .sum();

    let mut reviews = Vec::new();
    for skill in &skills {
        let skill_stats = stats.get(&skill.name).cloned().unwrap_or_default();
        let mut findings = Vec::new();

        if skill_stats.invoked == 0 {
            findings.push(Finding {
                kind: FindingKind::NeverUsed,
                severity: Severity::Info,
                message: "never invoked since usage tracking began".into(),
            });
        }

        if skill_stats.invoked > 0 && skill.verified_app_version < *app_version {
            findings.push(Finding {
                kind: FindingKind::StaleVerification,
                severity: Severity::Warning,
                message: format!(
                    "in use but verified against {}; current app is {app_version}",
                    skill.verified_app_version
                ),
            });
        }

        if skill_stats.invoked >= HIGH_FAILURE_MIN_INVOCATIONS
            && skill_stats.failure_rate() >= HIGH_FAILURE_RATE
        {
            findings.push(Finding {
                kind: FindingKind::HighFailureRate,
                severity: Severity::Warning,
                message: format!(
                    "{} of {} invocations failed ({:.0}%)",
                    skill_stats.failed,
                    skill_stats.invoked,
                    skill_stats.failure_rate() * 100.0
                ),
            });
        }

        let status = if findings.iter().any(|f| f.severity >= Severity::Warning) {
            ReviewStatus::Attention
        } else {
            ReviewStatus::Ok
        };
        reviews.push(SkillReview {
            skill: skill.name.clone(),
            status,
            stats: skill_stats,
            findings,
        });
    }

    Ok(ReviewReport {
        generated_at: Utc::now(),
        app_version: app_version.clone(),
        skills: reviews,
        orphan_events,
    })
}

/// Whether a daemon tick should re-run the review: when the app changed, or
/// when new usage lines arrived since the last review.
pub fn review_due(
    state: &DaemonState,
    log: &UsageLog,
    version_changed: bool,
    head_changed: bool,
) -> bool {
    version_changed || head_changed || state.usage_lines_reviewed != log.lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Config, Skill, UsageKind};
    use tempfile::TempDir;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    /// Two skills at app version 1.0.0; `web` verified, `api` never verified
    /// against anything newer than 0.9.0.
    fn setup() -> (TempDir, Registry) {
        let dir = TempDir::new().unwrap();
        let registry = Registry::init(dir.path(), Config::default()).unwrap();
        let web = Skill::new("web", v("1.0.0"), vec!["*".into()], v("1.0.0"));
        registry.add_skill(&web, "# Web").unwrap();
        let api = Skill::new("api", v("1.0.0"), vec!["*".into()], v("0.9.0"));
        registry.add_skill(&api, "# Api").unwrap();
        (dir, registry)
    }

    #[test]
    fn unused_skill_is_ok_with_info_finding() {
        let (_dir, registry) = setup();
        // The workspace seeds ask-skillastic on init, so three skills exist.
        let report = run(&registry, &v("1.0.0")).unwrap();
        assert_eq!(report.skills.len(), 3);
        let api = report.skills.iter().find(|s| s.skill == "api").unwrap();
        assert_eq!(api.status, ReviewStatus::Ok);
        assert!(
            api.findings
                .iter()
                .any(|f| f.kind == FindingKind::NeverUsed && f.severity == Severity::Info)
        );
    }

    #[test]
    fn stale_verification_and_high_failure_rate_flag_attention() {
        let (_dir, registry) = setup();
        // api: 4 invocations, 3 failed; app now at 2.0.0, api verified at 0.9.0.
        for _ in 0..4 {
            usage::record(&registry, "api", UsageKind::Invoked, None).unwrap();
        }
        for _ in 0..3 {
            usage::record(&registry, "api", UsageKind::Failed, None).unwrap();
        }
        usage::record(&registry, "web", UsageKind::Completed, None).unwrap();

        let report = run(&registry, &v("2.0.0")).unwrap();
        assert_eq!(report.attention_count(), 1);
        let api = report.skills.iter().find(|s| s.skill == "api").unwrap();
        assert_eq!(api.status, ReviewStatus::Attention);
        assert!(
            api.findings
                .iter()
                .any(|f| f.kind == FindingKind::StaleVerification)
        );
        assert!(
            api.findings
                .iter()
                .any(|f| f.kind == FindingKind::HighFailureRate)
        );
        // One completion does not count as an invocation for web.
        let web = report.skills.iter().find(|s| s.skill == "web").unwrap();
        assert_eq!(web.status, ReviewStatus::Ok);
        assert_eq!(web.stats.invoked, 0);
        assert_eq!(web.stats.completed, 1);
    }

    #[test]
    fn events_for_removed_skills_count_as_orphans() {
        let (dir, registry) = setup();
        usage::record(&registry, "api", UsageKind::Invoked, None).unwrap();
        registry.remove_skill("api").unwrap();
        let _ = dir; // keep tmpdir alive
        let report = run(&registry, &v("1.0.0")).unwrap();
        assert_eq!(report.orphan_events, 1);
        assert!(!report.skills.iter().any(|s| s.skill == "api"));
    }

    #[test]
    fn review_due_tracks_usage_lines_and_changes() {
        let (_dir, registry) = setup();
        let state = registry.state().unwrap();
        let empty = usage::load(&registry).unwrap();
        assert!(!review_due(&state, &empty, false, false));

        usage::record(&registry, "web", UsageKind::Invoked, None).unwrap();
        let log = usage::load(&registry).unwrap();
        assert!(review_due(&state, &log, false, false));

        // A state that already reviewed these lines is not due again...
        let mut caught_up = state.clone();
        caught_up.usage_lines_reviewed = log.lines;
        assert!(!review_due(&caught_up, &log, false, false));
        // ...unless the app version or HEAD moved this tick.
        assert!(review_due(&caught_up, &log, true, false));
        assert!(review_due(&caught_up, &log, false, true));
    }
}
