//! Core data model: skills, lineage, decisions, fingerprints.

use chrono::{DateTime, NaiveDate, Utc};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Who can invoke a skill.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillInvocation {
    /// Only the human types its name.
    UserInvoked,
    /// The model may reach for it automatically.
    ModelInvoked,
    /// Either the human or the model may invoke it.
    #[default]
    Both,
}

impl fmt::Display for SkillInvocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::UserInvoked => "user_invoked",
            Self::ModelInvoked => "model_invoked",
            Self::Both => "both",
        };
        f.write_str(s)
    }
}

fn default_bucket() -> String {
    "core".into()
}

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
    /// Who can invoke this skill.
    #[serde(default)]
    pub invocation: SkillInvocation,
    /// Other skills this skill depends on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    /// Bucket this skill belongs to, e.g. `core`, `engineering`, `productivity`.
    #[serde(default = "default_bucket")]
    pub bucket: String,
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
            invocation: SkillInvocation::default(),
            requires: Vec::new(),
            bucket: default_bucket(),
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
    /// Let the daemon run inference-powered refinement suggestions after a
    /// review that flags skills. Off by default; `suggest` always works
    /// when invoked explicitly.
    #[serde(default)]
    pub auto_suggest: bool,
    /// Model override for suggestions (Groq model id, or ignored by an
    /// external `llm_command` backend).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggest_model: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            app_name: "app".into(),
            app_version: None,
            auto_migrate: true,
            llm_command: None,
            auto_suggest: false,
            suggest_model: None,
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
    /// When the daemon last ran a skill-use review.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_review: Option<DateTime<Utc>>,
    /// When the daemon last attempted inference suggestions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_suggest: Option<DateTime<Utc>>,
    /// Usage-log line count at the last review, so ticks only re-review
    /// when new events arrived.
    #[serde(default)]
    pub usage_lines_reviewed: u64,
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

/// One recorded use of a skill, appended to `.skillastic/usage.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageEvent {
    pub timestamp: DateTime<Utc>,
    pub skill: String,
    pub event: UsageKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageKind {
    Invoked,
    Completed,
    Failed,
}

/// Aggregated use of one skill over every recorded event.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageStats {
    pub invoked: u64,
    pub completed: u64,
    pub failed: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_seen: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen: Option<DateTime<Utc>>,
}

impl UsageStats {
    /// Failed uses per invoked use, 0.0 when never invoked.
    pub fn failure_rate(&self) -> f64 {
        if self.invoked == 0 {
            0.0
        } else {
            self.failed as f64 / self.invoked as f64
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

/// A deterministic observation about one skill's usage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub kind: FindingKind,
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    /// The skill has never been invoked.
    NeverUsed,
    /// The skill is in use but was verified against an older app version.
    StaleVerification,
    /// At least half the recorded invocations failed.
    HighFailureRate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    Ok,
    Attention,
}

impl fmt::Display for ReviewStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Ok => "ok",
            Self::Attention => "attention",
        };
        f.write_str(s)
    }
}

impl fmt::Display for UsageKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Invoked => "invoked",
            Self::Completed => "completed",
            Self::Failed => "failed",
        };
        f.write_str(s)
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Critical => "critical",
        };
        f.write_str(s)
    }
}

/// Review outcome for a single skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillReview {
    pub skill: String,
    pub status: ReviewStatus,
    pub stats: UsageStats,
    #[serde(default)]
    pub findings: Vec<Finding>,
}

/// `.skillastic/review.json` — outcome of the latest skill-use review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewReport {
    pub generated_at: DateTime<Utc>,
    pub app_version: Version,
    #[serde(default)]
    pub skills: Vec<SkillReview>,
    /// Events referencing skills that are no longer registered.
    #[serde(default)]
    pub orphan_events: u64,
}

impl Default for ReviewReport {
    fn default() -> Self {
        Self {
            generated_at: DateTime::<Utc>::from(std::time::UNIX_EPOCH),
            app_version: Version::new(0, 0, 0),
            skills: Vec::new(),
            orphan_events: 0,
        }
    }
}

impl ReviewReport {
    pub fn attention_count(&self) -> usize {
        self.skills
            .iter()
            .filter(|s| s.status == ReviewStatus::Attention)
            .count()
    }
}

/// One proposed refinement for a skill, produced by inference.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestionItem {
    pub title: String,
    pub rationale: String,
    /// Advisory edit guidance; never applied to the body automatically.
    pub suggested_edit: String,
}

/// Suggestions for one skill from one inference run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSuggestions {
    pub skill: String,
    pub generated_at: DateTime<Utc>,
    pub backend: String,
    pub model: String,
    pub status: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub items: Vec<SuggestionItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `.skillastic/suggestions.json` — latest suggestions per skill.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SuggestionStore {
    #[serde(default)]
    pub suggestions: Vec<SkillSuggestions>,
}

impl SuggestionStore {
    /// Replace the entry for `entry.skill`, or prepend it as the newest.
    pub fn upsert(&mut self, entry: SkillSuggestions) {
        self.suggestions.retain(|s| s.skill != entry.skill);
        self.suggestions.insert(0, entry);
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
        assert_eq!(back.invocation, SkillInvocation::Both);
        assert_eq!(back.bucket, "core");
        assert!(back.requires.is_empty());
    }

    #[test]
    fn skill_defaults_for_backward_compatibility() {
        let old = r#"{
            "name": "legacy",
            "skill_version": "1.0.0",
            "compatible_apps": [],
            "created": "2024-01-01T00:00:00Z",
            "confidence": 0.5,
            "status": "needs_validation",
            "verified_app_version": "1.0.0",
            "body_path": "legacy.md"
        }"#;
        let skill: Skill = serde_json::from_str(old).unwrap();
        assert_eq!(skill.invocation, SkillInvocation::Both);
        assert_eq!(skill.bucket, "core");
        assert!(skill.requires.is_empty());
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
