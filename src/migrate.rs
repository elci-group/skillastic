//! Skill Migration Engine: the deterministic pipeline that brings a skill
//! from its verified app version to the current one.
//!
//! Pipeline: snapshot → commit archaeology → context capture → delta →
//! append-only patch (or optional LLM rewrite) → version bump + lineage.

use crate::archaeology::{Archaeology, CommitChain};
use crate::capture::Capture;
use crate::delta::ContextDelta;
use crate::error::{Result, SkillasticError};
use crate::git::Git;
use crate::model::{MutationRecord, Skill, SkillStatus, VersionDelta};
use crate::registry::Registry;
use chrono::Utc;
use semver::Version;
use serde::Serialize;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Confidence assigned after an unreviewed auto-migration.
pub const MIGRATED_CONFIDENCE: f64 = 0.70;
/// Confidence assigned by `skillastic verify`.
pub const VERIFIED_CONFIDENCE: f64 = 0.95;

#[derive(Debug, Serialize)]
pub struct MigrationOutcome {
    pub skill: String,
    pub from_skill_version: Version,
    pub to_skill_version: Version,
    pub parent: String,
    pub from_app: Version,
    pub to_app: Version,
    pub delta: ContextDelta,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_dir: Option<PathBuf>,
    pub llm_used: bool,
    pub dry_run: bool,
}

pub struct Migrator<'a> {
    registry: &'a Registry,
}

impl<'a> Migrator<'a> {
    pub fn new(registry: &'a Registry) -> Self {
        Self { registry }
    }

    /// Run the migration pipeline for one skill. With `dry_run`, computes
    /// the full outcome but writes nothing (no snapshot, no saves).
    pub fn migrate(&self, name: &str, to_app: &Version, dry_run: bool) -> Result<MigrationOutcome> {
        let mut skill = self.registry.load_skill(name)?;
        let body = self.registry.skill_body(&skill)?;
        let from_app = skill.verified_app_version.clone();
        let bump = VersionDelta::between(&from_app, to_app);
        if bump == VersionDelta::Same {
            return Err(SkillasticError::Other(format!(
                "skill '{name}' already targets app {to_app}; nothing to migrate"
            )));
        }
        let project_root = self.registry.project_root();

        // 1. Snapshot the existing skill (spec: capture instructions,
        //    examples, failure modes, tool assumptions, prior mutations).
        let snapshot_dir = if dry_run {
            None
        } else {
            Some(self.registry.snapshot(&skill)?)
        };

        // 2. Commit-chain analysis (best effort; capture-only fallback when
        //    the project isn't a git repo or refs don't resolve).
        let chain = self.commit_chain(&project_root, &from_app, to_app);

        // 3. Context delta generation inputs: old fingerprint from the git
        //    ref when available, else the fingerprint stored on the skill.
        let new_fp = Capture::scan(&project_root)?;
        let old_fp = match &chain {
            Some(c) => Git::open(&project_root)
                .and_then(|git| Capture::scan_ref(&git, &c.from_ref))
                .unwrap_or_else(|_| skill.context.clone()),
            None => skill.context.clone(),
        };

        let delta = ContextDelta::build(
            chain.as_ref(),
            from_app.clone(),
            to_app.clone(),
            &old_fp,
            &new_fp,
        );

        // 4. Patch: deterministic append-only notes, or the optional LLM
        //    adapter when `llm_command` is configured.
        let config = self.registry.config()?;
        let (new_body, llm_used) = match config.llm_command.as_deref() {
            Some(cmd) => (LlmAdapter::new(cmd).rewrite(&skill, &body, &delta)?, true),
            None => (format!("{body}{}", delta.to_markdown()), false),
        };

        // 5. Version bump + lineage.
        let from_skill_version = skill.skill_version.clone();
        let to_skill_version = bump.bump_skill(&from_skill_version);
        let parent = skill.id();
        let commit_ref = chain
            .as_ref()
            .map(|c| format!("{}..{}", c.from_ref, c.to_ref))
            .or_else(|| Git::open(&project_root).ok().and_then(|g| g.head().ok()))
            .unwrap_or_else(|| "unknown".into());

        if !dry_run {
            skill.parent = Some(parent.clone());
            skill.skill_version = to_skill_version.clone();
            skill.verified_app_version = to_app.clone();
            skill.context = new_fp;
            skill.confidence = MIGRATED_CONFIDENCE;
            skill.status = SkillStatus::NeedsValidation;
            skill.mutation_history.push(MutationRecord {
                commit: commit_ref,
                reason: delta.reason(),
                timestamp: Utc::now(),
                from_app: Some(from_app.clone()),
                to_app: Some(to_app.clone()),
            });
            self.registry.save_skill(&skill)?;
            self.registry.save_body(&skill, &new_body)?;
        }

        Ok(MigrationOutcome {
            skill: name.to_string(),
            from_skill_version,
            to_skill_version,
            parent,
            from_app,
            to_app: to_app.clone(),
            delta,
            snapshot_dir,
            llm_used,
            dry_run,
        })
    }

    fn commit_chain(
        &self,
        project_root: &Path,
        from_app: &Version,
        to_app: &Version,
    ) -> Option<CommitChain> {
        let arch = Archaeology::new(project_root).ok()?;
        let from_ref = arch.git().resolve_ref(&from_app.to_string())?;
        let to_ref = arch
            .git()
            .resolve_ref(&to_app.to_string())
            .or_else(|| arch.git().resolve_ref("HEAD"))?;
        arch.analyze_refs(&from_ref, &to_ref).ok()
    }
}

/// Mark a skill verified against the current app version.
pub fn verify(registry: &Registry, name: &str, app_version: &Version) -> Result<Skill> {
    let mut skill = registry.load_skill(name)?;
    skill.last_verified = Some(Utc::now().date_naive());
    skill.confidence = VERIFIED_CONFIDENCE;
    skill.status = SkillStatus::Active;
    skill.verified_app_version = app_version.clone();
    registry.save_skill(&skill)?;
    Ok(skill)
}

/// Optional external rewriter. The command receives a JSON payload on
/// stdin (`{skill, skill_version, old_body, delta}`) and must print the
/// new body on stdout. Off unless `llm_command` is set in the config.
pub struct LlmAdapter<'a> {
    command: &'a str,
}

impl<'a> LlmAdapter<'a> {
    pub fn new(command: &'a str) -> Self {
        Self { command }
    }

    pub fn rewrite(&self, skill: &Skill, old_body: &str, delta: &ContextDelta) -> Result<String> {
        let payload = serde_json::json!({
            "skill": skill.name,
            "skill_version": skill.skill_version.to_string(),
            "old_body": old_body,
            "delta": delta,
        });
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(self.command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child
            .stdin
            .take()
            .expect("stdin was piped")
            .write_all(payload.to_string().as_bytes())?;
        let out = child.wait_with_output()?;
        if !out.status.success() {
            return Err(SkillasticError::Llm(format!(
                "'{}' exited with {}: {}",
                self.command,
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        let body = String::from_utf8_lossy(&out.stdout).into_owned();
        if body.trim().is_empty() {
            return Err(SkillasticError::Llm(format!(
                "'{}' produced an empty body",
                self.command
            )));
        }
        Ok(body)
    }
}
