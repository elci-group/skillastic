//! Skillastic daemon: polls the application for version/HEAD changes and
//! re-resolves all skills, auto-migrating when configured. Also reviews
//! skill-use events each tick and, when opted in, refreshes inference
//! refinement suggestions for flagged skills.

use crate::appver;
use crate::archaeology::Archaeology;
use crate::error::Result;
use crate::git::Git;
use crate::migrate::Migrator;
use crate::model::{Decision, ReviewStatus};
use crate::registry::Registry;
use crate::resolver::Resolver;
use crate::review;
use crate::suggest::{self, Backend};
use crate::usage;
use chrono::Utc;
use std::time::Duration;

/// Minimum seconds between daemon-driven suggestion runs.
const SUGGEST_COOLDOWN_SECS: i64 = 3600;

pub struct Daemon<'a> {
    registry: &'a Registry,
    interval: Duration,
}

impl<'a> Daemon<'a> {
    pub fn new(registry: &'a Registry, interval_secs: u64) -> Self {
        Self {
            registry,
            interval: Duration::from_secs(interval_secs.max(1)),
        }
    }

    /// Poll forever. Default SIGINT handling terminates the process.
    pub fn run(&self) -> Result<()> {
        println!(
            "skillastic daemon: watching {} every {}s",
            self.registry.project_root().display(),
            self.interval.as_secs()
        );
        loop {
            for event in self.tick()? {
                println!("[{}] {event}", Utc::now().format("%Y-%m-%d %H:%M:%S"));
            }
            std::thread::sleep(self.interval);
        }
    }

    /// One poll cycle. Returns the events that fired (empty when nothing
    /// changed). Kept separate from `run` for testability.
    pub fn tick(&self) -> Result<Vec<String>> {
        let mut events = Vec::new();
        let config = self.registry.config()?;
        let project_root = self.registry.project_root();
        let app_version = match appver::detect(&project_root, &config, None) {
            Ok(v) => v,
            Err(_) => return Ok(events), // undetectable version; try again next tick
        };
        let head = Git::open(&project_root).ok().and_then(|g| g.head().ok());
        let mut state = self.registry.state()?;

        let version_changed = state.last_app_version.as_ref() != Some(&app_version);
        let head_changed = head.is_some() && state.last_head.as_deref() != head.as_deref();

        if version_changed || head_changed {
            events.push(format!(
                "change detected: app {} -> {app_version}, head {:?} -> {:?}",
                state
                    .last_app_version
                    .as_ref()
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "?".into()),
                state.last_head,
                head
            ));

            let skills = self.registry.list_skills()?;
            if !skills.is_empty() {
                let arch = Archaeology::new(&project_root).ok();
                let resolver = Resolver::new(arch.as_ref(), &app_version);
                for res in resolver.resolve_all(&skills, &app_version)? {
                    events.push(format!(
                        "resolve {}: {} ({})",
                        res.skill, res.decision, res.reason
                    ));
                    self.registry.apply_resolution(&res)?;
                    if res.decision == Decision::Migrate && config.auto_migrate {
                        match Migrator::new(self.registry).migrate(&res.skill, &app_version, false)
                        {
                            Ok(outcome) => events.push(format!(
                                "migrated {}: skill {} -> {}",
                                outcome.skill, outcome.from_skill_version, outcome.to_skill_version
                            )),
                            Err(e) => {
                                events.push(format!("migration failed for {}: {e}", res.skill))
                            }
                        }
                    }
                }
            }

            state.last_app_version = Some(app_version.clone());
            state.last_head = head;
        }

        // Phase 2: skill-use review, then optional inference suggestions.
        // Errors are caught into events (like migration failures) so the
        // loop never dies on a bad review or backend.
        let usage_log = usage::load(self.registry)?;
        if review::review_due(&state, &usage_log, version_changed, head_changed) {
            match review::run(self.registry, &app_version) {
                Ok(report) => match self.registry.save_review(&report) {
                    Ok(()) => {
                        state.last_review = Some(Utc::now());
                        state.usage_lines_reviewed = usage_log.lines;
                        let flagged: Vec<String> = report
                            .skills
                            .iter()
                            .filter(|s| s.status == ReviewStatus::Attention)
                            .map(|s| s.skill.clone())
                            .collect();
                        if !flagged.is_empty() {
                            events.push(format!(
                                "review: {} skill(s) need attention ({})",
                                flagged.len(),
                                flagged.join(", ")
                            ));
                            if config.auto_suggest && suggest_cooled_down(&state) {
                                let backend = suggest::resolve_backend(&config);
                                let targets: Vec<crate::model::Skill> = flagged
                                    .iter()
                                    .filter_map(|name| self.registry.load_skill(name).ok())
                                    .collect();
                                match suggest::run(
                                    self.registry,
                                    &app_version,
                                    &targets,
                                    &report,
                                    None,
                                    &backend,
                                ) {
                                    Ok(outcome) => {
                                        state.last_suggest = Some(Utc::now());
                                        match backend {
                                            Backend::None => events.push(
                                                "suggestions skipped: no backend configured".into(),
                                            ),
                                            _ => events.push(format!(
                                                "suggestions refreshed for {} skill(s) via {}",
                                                outcome.attempted,
                                                backend_name(&backend)
                                            )),
                                        }
                                    }
                                    Err(e) => {
                                        events.push(format!("suggestions failed: {e}"));
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => events.push(format!("review failed: {e}")),
                },
                Err(e) => events.push(format!("review failed: {e}")),
            }
        }

        state.last_check = Some(Utc::now());
        for event in &events {
            state.log(format!(
                "[{}] {event}",
                Utc::now().format("%Y-%m-%d %H:%M:%S")
            ));
        }
        self.registry.save_state(&state)?;
        Ok(events)
    }
}

fn suggest_cooled_down(state: &crate::model::DaemonState) -> bool {
    state
        .last_suggest
        .map(|last| (Utc::now() - last).num_seconds() >= SUGGEST_COOLDOWN_SECS)
        .unwrap_or(true)
}

fn backend_name(backend: &Backend) -> &'static str {
    match backend {
        Backend::None => "none",
        Backend::LlmCommand(_) => "llm_command",
        Backend::Groq => "groq",
    }
}

/// Recent daemon events from `state.json`, for inspection.
pub fn recent_events(registry: &Registry) -> Result<Vec<String>> {
    Ok(registry.state()?.events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Config, Skill, SkillStatus, UsageKind};
    use semver::Version;
    use std::fs;
    use tempfile::TempDir;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    /// A temp project whose version comes from its Cargo.toml.
    fn setup(app_version: &str) -> (TempDir, Registry) {
        let dir = TempDir::new().unwrap();
        set_app_version(&dir, app_version);
        let registry = Registry::init(dir.path(), Config::default()).unwrap();
        (dir, registry)
    }

    fn set_app_version(dir: &TempDir, version: &str) {
        fs::write(
            dir.path().join("Cargo.toml"),
            format!("[package]\nname = \"t\"\nversion = \"{version}\"\n"),
        )
        .unwrap();
    }

    fn add_skill(registry: &Registry, range: &str, verified: &str) {
        let skill = Skill::new("web", v("1.0.0"), vec![range.into()], v(verified));
        registry.add_skill(&skill, "# Web\nUse the thing.").unwrap();
    }

    #[test]
    fn first_tick_establishes_baseline() {
        let (_dir, registry) = setup("1.0.0");
        add_skill(&registry, ">=1.0.0, <2.0.0", "1.0.0");
        let events = Daemon::new(&registry, 1).tick().unwrap();

        assert!(events.iter().any(|e| e.contains("change detected")));
        assert!(events.iter().any(|e| e.contains("resolve web: load")));
        let state = registry.state().unwrap();
        assert_eq!(state.last_app_version, Some(v("1.0.0")));
        assert!(state.last_check.is_some());
        assert_eq!(state.events.len(), events.len());
    }

    #[test]
    fn steady_state_tick_is_quiet() {
        let (_dir, registry) = setup("1.0.0");
        let daemon = Daemon::new(&registry, 1);
        assert!(!daemon.tick().unwrap().is_empty());
        assert!(daemon.tick().unwrap().is_empty());
    }

    #[test]
    fn version_bump_re_resolves_without_migrating() {
        let (dir, registry) = setup("1.0.0");
        add_skill(&registry, ">=1.0.0, <2.0.0", "1.0.0");
        let daemon = Daemon::new(&registry, 1);
        daemon.tick().unwrap();

        set_app_version(&dir, "1.1.0");
        let events = daemon.tick().unwrap();
        assert!(events.iter().any(|e| e.contains("resolve web: validate")));
        assert!(!events.iter().any(|e| e.contains("migrated")));
        // Minor bump: resolver only asks for validation; version untouched,
        // but the stored status reflects the decision.
        let skill = registry.load_skill("web").unwrap();
        assert_eq!(skill.verified_app_version, v("1.0.0"));
        assert_eq!(skill.status, SkillStatus::NeedsValidation);
    }

    #[test]
    fn auto_migrates_out_of_range_skills() {
        let (dir, registry) = setup("1.0.0");
        add_skill(&registry, ">=1.0.0, <2.0.0", "1.0.0");
        let daemon = Daemon::new(&registry, 1);
        daemon.tick().unwrap();

        set_app_version(&dir, "2.0.0");
        let events = daemon.tick().unwrap();
        assert!(events.iter().any(|e| e.contains("resolve web: migrate")));
        assert!(
            events
                .iter()
                .any(|e| e.starts_with("migrated web: skill 1.0.0 -> 2.0.0"))
        );

        let skill = registry.load_skill("web").unwrap();
        assert_eq!(skill.skill_version, v("2.0.0"));
        assert_eq!(skill.verified_app_version, v("2.0.0"));
        assert_eq!(registry.snapshots_for("web").unwrap(), vec!["web@1.0.0"]);
    }

    #[test]
    fn auto_migrate_disabled_leaves_skill_alone() {
        let (dir, registry) = setup("1.0.0");
        add_skill(&registry, ">=1.0.0, <2.0.0", "1.0.0");
        let mut config = registry.config().unwrap();
        config.auto_migrate = false;
        registry.save_config(&config).unwrap();
        let daemon = Daemon::new(&registry, 1);
        daemon.tick().unwrap();

        set_app_version(&dir, "2.0.0");
        let events = daemon.tick().unwrap();
        assert!(events.iter().any(|e| e.contains("resolve web: migrate")));
        assert!(!events.iter().any(|e| e.contains("migrated")));
        let skill = registry.load_skill("web").unwrap();
        assert_eq!(skill.verified_app_version, v("1.0.0"));
        assert_eq!(skill.status, SkillStatus::NeedsMigration);
    }

    #[test]
    fn recent_events_reads_state_log() {
        let (_dir, registry) = setup("1.0.0");
        let events = Daemon::new(&registry, 1).tick().unwrap();
        assert_eq!(recent_events(&registry).unwrap().len(), events.len());
    }

    #[test]
    fn tick_reviews_usage_and_persists_report() {
        let (_dir, registry) = setup("1.0.0");
        add_skill(&registry, ">=1.0.0, <2.0.0", "1.0.0");
        usage::record(&registry, "web", UsageKind::Invoked, None).unwrap();

        let events = Daemon::new(&registry, 1).tick().unwrap();
        // The skill is in use and verified: no attention event.
        assert!(!events.iter().any(|e| e.starts_with("review:")));

        let state = registry.state().unwrap();
        assert_eq!(state.usage_lines_reviewed, 1);
        assert!(state.last_review.is_some());
        let report = registry.review().unwrap();
        // web plus the seeded ask-skillastic.
        assert_eq!(report.skills.len(), 2);
        assert_eq!(report.skills[0].skill, "ask-skillastic");

        // New tick, no new events, no changes: quiet, no re-review.
        assert!(Daemon::new(&registry, 1).tick().unwrap().is_empty());
    }

    #[test]
    fn flagged_skill_logs_event_without_auto_suggest() {
        let (_dir, registry) = setup("1.0.0");
        add_skill(&registry, ">=1.0.0, <2.0.0", "1.0.0");
        for _ in 0..3 {
            usage::record(&registry, "web", UsageKind::Invoked, None).unwrap();
        }
        for _ in 0..2 {
            usage::record(&registry, "web", UsageKind::Failed, None).unwrap();
        }

        let events = Daemon::new(&registry, 1).tick().unwrap();
        assert!(
            events
                .iter()
                .any(|e| e.contains("review: 1 skill(s) need attention"))
        );
        // auto_suggest is off by default: no suggestion attempt.
        assert!(!registry.root().join("suggestions.json").exists());
        assert!(registry.state().unwrap().last_suggest.is_none());
    }

    #[test]
    fn auto_suggest_refreshes_via_configured_command() {
        let (_dir, registry) = setup("1.0.0");
        add_skill(&registry, ">=1.0.0, <2.0.0", "1.0.0");
        for _ in 0..3 {
            usage::record(&registry, "web", UsageKind::Invoked, None).unwrap();
        }
        for _ in 0..3 {
            usage::record(&registry, "web", UsageKind::Failed, None).unwrap();
        }
        let mut config = registry.config().unwrap();
        config.auto_suggest = true;
        config.llm_command = Some("printf '{\"summary\":\"tighten\",\"items\":[]}'".into());
        registry.save_config(&config).unwrap();

        let events = Daemon::new(&registry, 1).tick().unwrap();
        assert!(
            events
                .iter()
                .any(|e| e.contains("suggestions refreshed for 1 skill(s) via llm_command"))
        );
        assert!(registry.state().unwrap().last_suggest.is_some());
        let entry = registry
            .suggestions()
            .unwrap()
            .suggestions
            .into_iter()
            .find(|s| s.skill == "web")
            .unwrap();
        assert_eq!(entry.status, "complete");
        assert_eq!(entry.summary, "tighten");
    }
}
