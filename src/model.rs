//! Core data model: skills, lineage, decisions, fingerprints.

use chrono::{DateTime, NaiveDate, Utc};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// A living skill object with lineage, per the Skillastic spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub skill_version: Version,
    /// Semver requirement strings this skill is compatible with,
    /// e.g. `">=2.1.0, <3.0.0"`.
    pub compatible_apps: Vec<String>,
    /// Lineage pointer: `"<name>@<skill_version>"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub created: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_verified: Option<NaiveDate>,
    /// 0.0–1.0. Auto-migration sets 0.70; `skillastic verify` sets 0.95.
    pub confidence: f64,
    pub status: SkillStatus,
    /// App version this skill was last verified/migrated against.
    pub verified_app_version: Version,
    #[serde(default)]
    pub mutation_history: Vec<MutationRecord>,
    #[serde(default)]
    pub context: ContextFingerprint,
    /// Path of the instruction body, relative to `.skillastic/skills/`.
    pub body_path: String,
}

impl Skill {
    pub fn new(
        name: impl Into<String>,
        skill_version: Version,
        compatible_apps: Vec<String>,
        verified_app_version: Version,
    ) -> Self {
        let name = name.into();
        Self {
            body_path: format!("{name}.md"),
            name,
            skill_version,
            compatible_apps,
            parent: None,
            created: Utc::now(),
            last_verified: None,
            confidence: 0.50,
            status: SkillStatus::NeedsValidation,
            verified_app_version,
            mutation_history: Vec::new(),
            context: ContextFingerprint::default(),
        }
    }

    /// Stable identifier used in lineage pointers and snapshot dirs.
    pub fn id(&self) -> String {
        format!("{}@{}", self.name, self.skill_version)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillStatus {
    Active,
    NeedsValidation,
    NeedsMigration,
    Incompatible,
}

impl fmt::Display for SkillStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Active => "active",
            Self::NeedsValidation => "needs_validation",
            Self::NeedsMigration => "needs_migration",
            Self::Incompatible => "incompatible",
        };
        f.write_str(s)
    }
}

/// One entry in a skill's evolutionary history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationRecord {
    /// Commit SHA or range (`old..new`) that motivated the mutation.
    pub commit: String,
    pub reason: String,
    pub timestamp: DateTime<Utc>,
    /// App version window the mutation bridges.
    pub from_app: Option<Version>,
    pub to_app: Option<Version>,
}

/// Deterministic resolver verdict (spec's compatibility table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// App patch update — load the skill.
    Load,
    /// App minor update — load, but flag for validation.
    Validate,
    /// App major update / out of range — run the migration pipeline.
    Migrate,
    /// Dependency change not explained by the bump level, or
    /// unresolvable refs — run deep analysis.
    DeepAnalysis,
    /// No lineage and no git history to migrate from.
    Incompatible,
}

impl fmt::Display for Decision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Load => "load",
            Self::Validate => "validate",
            Self::Migrate => "migrate",
            Self::DeepAnalysis => "deep_analysis",
            Self::Incompatible => "incompatible",
        };
        f.write_str(s)
    }
}

/// One skill's resolution against the current app version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Resolution {
    pub skill: String,
    pub from_app: Version,
    pub to_app: Version,
    pub decision: Decision,
    pub reason: String,
}

/// Semver distance between two app versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionDelta {
    Same,
    Patch,
    Minor,
    Major,
    Downgrade,
}

impl VersionDelta {
    pub fn between(from: &Version, to: &Version) -> Self {
        if to == from {
            Self::Same
        } else if to < from {
            Self::Downgrade
        } else if to.major != from.major {
            Self::Major
        } else if to.minor != from.minor {
            Self::Minor
        } else {
            Self::Patch
        }
    }

    /// Bump a skill version by the same level as the app bump.
    pub fn bump_skill(&self, v: &Version) -> Version {
        let mut next = v.clone();
        match self {
            Self::Major => {
                next.major += 1;
                next.minor = 0;
                next.patch = 0;
            }
            Self::Minor => {
                next.minor += 1;
                next.patch = 0;
            }
            // Same/Patch/Downgrade all advance the skill by a patch:
            // the skill changed even if the app barely moved.
            _ => next.patch += 1,
        }
        next.pre = semver::Prerelease::EMPTY;
        next.build = semver::BuildMetadata::EMPTY;
        next
    }
}

/// Snapshot of what a codebase actually is, captured at a point in time.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextFingerprint {
    /// Inferred frameworks, e.g. "next.js", "react", "next.js app-router".
    #[serde(default)]
    pub frameworks: Vec<String>,
    /// Build/toolchain markers, e.g. "vite", "webpack", "typescript".
    #[serde(default)]
    pub toolchains: Vec<String>,
    /// Notable top-level directories, e.g. "app", "pages", "src".
    #[serde(default)]
    pub directories: Vec<String>,
    /// Dependency name → version requirement, from package manifests.
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
}

impl ContextFingerprint {
    pub fn is_empty(&self) -> bool {
        self.frameworks.is_empty()
            && self.toolchains.is_empty()
            && self.directories.is_empty()
            && self.dependencies.is_empty()
    }
}

/// `.skillastic/config.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub app_name: String,
    /// Pinned app version; when absent, auto-detect from manifests/git tags.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_version: Option<Version>,
    pub auto_migrate: bool,
    /// Optional external command for LLM-assisted body rewrites.
    /// Receives `{old_body, delta}` JSON on stdin, returns the new body
    /// on stdout. Off (None) by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_command: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            app_name: "app".into(),
            app_version: None,
            auto_migrate: true,
            llm_command: None,
        }
    }
}

/// `.skillastic/state.json` — daemon bookkeeping.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DaemonState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_app_version: Option<Version>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_head: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_check: Option<DateTime<Utc>>,
    /// Recent daemon events (ring buffer, newest last).
    #[serde(default)]
    pub events: Vec<String>,
}

impl DaemonState {
    const MAX_EVENTS: usize = 50;

    pub fn log(&mut self, event: impl Into<String>) {
        self.events.push(event.into());
        if self.events.len() > Self::MAX_EVENTS {
            let excess = self.events.len() - Self::MAX_EVENTS;
            self.events.drain(..excess);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn version_delta_levels() {
        assert_eq!(
            VersionDelta::between(&v("2.4.1"), &v("2.4.1")),
            VersionDelta::Same
        );
        assert_eq!(
            VersionDelta::between(&v("2.4.1"), &v("2.4.2")),
            VersionDelta::Patch
        );
        assert_eq!(
            VersionDelta::between(&v("2.4.1"), &v("2.5.0")),
            VersionDelta::Minor
        );
        assert_eq!(
            VersionDelta::between(&v("2.4.1"), &v("3.0.0")),
            VersionDelta::Major
        );
        assert_eq!(
            VersionDelta::between(&v("3.0.0"), &v("2.4.1")),
            VersionDelta::Downgrade
        );
    }

    #[test]
    fn skill_bump_follows_app_bump() {
        assert_eq!(VersionDelta::Major.bump_skill(&v("2.4.1")), v("3.0.0"));
        assert_eq!(VersionDelta::Minor.bump_skill(&v("2.4.1")), v("2.5.0"));
        assert_eq!(VersionDelta::Patch.bump_skill(&v("2.4.1")), v("2.4.2"));
    }

    #[test]
    fn skill_json_roundtrip() {
        let mut skill = Skill::new(
            "frontend-react",
            v("2.4.1"),
            vec![">=2.1.0, <3.0.0".into()],
            v("2.4.1"),
        );
        skill.parent = Some("frontend-react@2.3.0".into());
        skill.confidence = 0.94;
        skill.mutation_history.push(MutationRecord {
            commit: "a91f3e".into(),
            reason: "Added vector indexing".into(),
            timestamp: Utc::now(),
            from_app: Some(v("2.3.0")),
            to_app: Some(v("2.4.0")),
        });

        let json = serde_json::to_string_pretty(&skill).unwrap();
        let back: Skill = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id(), "frontend-react@2.4.1");
        assert_eq!(back.parent.as_deref(), Some("frontend-react@2.3.0"));
        assert_eq!(back.mutation_history.len(), 1);
        assert_eq!(back.status, SkillStatus::NeedsValidation);
    }

    #[test]
    fn daemon_state_ring_buffer() {
        let mut state = DaemonState::default();
        for i in 0..60 {
            state.log(format!("event {i}"));
        }
        assert_eq!(state.events.len(), DaemonState::MAX_EVENTS);
        assert_eq!(state.events.last().unwrap(), "event 59");
    }
}
