//! Skillastic daemon: polls the application for version/HEAD changes and
//! re-resolves all skills, auto-migrating when configured.

use crate::appver;
use crate::archaeology::Archaeology;
use crate::error::Result;
use crate::git::Git;
use crate::migrate::Migrator;
use crate::model::Decision;
use crate::registry::Registry;
use crate::resolver::Resolver;
use chrono::Utc;
use std::time::Duration;

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
                    events.push(format!("resolve {}: {} ({})", res.skill, res.decision, res.reason));
                    self.registry.apply_resolution(&res)?;
                    if res.decision == Decision::Migrate && config.auto_migrate {
                        match Migrator::new(self.registry).migrate(&res.skill, &app_version, false) {
                            Ok(outcome) => events.push(format!(
                                "migrated {}: skill {} -> {}",
                                outcome.skill, outcome.from_skill_version, outcome.to_skill_version
                            )),
                            Err(e) => events.push(format!("migration failed for {}: {e}", res.skill)),
                        }
                    }
                }
            }

            state.last_app_version = Some(app_version);
            state.last_head = head;
        }

        state.last_check = Some(Utc::now());
        for event in &events {
            state.log(format!("[{}] {event}", Utc::now().format("%Y-%m-%d %H:%M:%S")));
        }
        self.registry.save_state(&state)?;
        Ok(events)
    }
}

/// Recent daemon events from `state.json`, for inspection.
pub fn recent_events(registry: &Registry) -> Result<Vec<String>> {
    Ok(registry.state()?.events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Config, Skill, SkillStatus};
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
        assert!(events.iter().any(|e| e.starts_with("migrated web: skill 1.0.0 -> 2.0.0")));

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
}
