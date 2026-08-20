//! Skill Registry: the on-disk `.skillastic/` workspace.
//!
//! Layout:
//! ```text
//! .skillastic/
//!   config.json                 workspace config
//!   state.json                  daemon state
//!   promoted.json               curated promoted skill names
//!   skills/<bucket>/<name>/     meta.json + body.md (new layout)
//!   skills/<name>.json          legacy skill object
//!   skills/<name>.md            legacy instruction body
//!   snapshots/<name>@<ver>/     frozen skill.json + body.md per migration
//! ```

use crate::error::{Result, SkillasticError};
use crate::model::{
    Config, DaemonState, Decision, Resolution, Skill, SkillInvocation, SkillStatus,
};
use crate::templates;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const DIR: &str = ".skillastic";
const SKILLS: &str = "skills";
const SNAPSHOTS: &str = "snapshots";
const CONFIG: &str = "config.json";
const STATE: &str = "state.json";
const PROMOTED: &str = "promoted.json";
pub const BUCKETS: &[&str] = &[
    "core",
    "engineering",
    "productivity",
    "misc",
    "in-progress",
    "deprecated",
];

pub struct Registry {
    root: PathBuf,
}

impl Registry {
    /// Create a fresh workspace. Fails if one already exists.
    pub fn init(project_root: &Path, config: Config) -> Result<Self> {
        let root = project_root.join(DIR);
        if root.exists() {
            return Err(SkillasticError::Other(format!(
                "workspace already exists at {}",
                root.display()
            )));
        }
        fs::create_dir_all(root.join(SKILLS))?;
        for bucket in BUCKETS {
            fs::create_dir_all(root.join(SKILLS).join(bucket))?;
        }
        fs::create_dir_all(root.join(SNAPSHOTS))?;
        let registry = Self { root };
        registry.save_config(&config)?;
        registry.save_state(&DaemonState::default())?;
        registry.save_promoted(&PromotedSet {
            skills: vec!["ask-skillastic".into()],
        })?;
        registry.seed_ask_skillastic()?;
        Ok(registry)
    }

    /// Open an existing workspace.
    pub fn open(project_root: &Path) -> Result<Self> {
        let root = project_root.join(DIR);
        if !root.is_dir() {
            return Err(SkillasticError::NotInitialized(root.display().to_string()));
        }
        Ok(Self { root })
    }

    pub fn exists(project_root: &Path) -> bool {
        project_root.join(DIR).is_dir()
    }

    /// Path to the `.skillastic/` directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Path to the project that contains this workspace.
    pub fn project_root(&self) -> PathBuf {
        self.root
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
    }

    // ---- config & state -------------------------------------------------

    pub fn config(&self) -> Result<Config> {
        let raw = fs::read_to_string(self.root.join(CONFIG))?;
        Ok(serde_json::from_str(&raw)?)
    }

    pub fn save_config(&self, config: &Config) -> Result<()> {
        write_atomic(
            &self.root.join(CONFIG),
            &serde_json::to_string_pretty(config)?,
        )
    }

    pub fn state(&self) -> Result<DaemonState> {
        let path = self.root.join(STATE);
        if !path.exists() {
            return Ok(DaemonState::default());
        }
        let raw = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    pub fn save_state(&self, state: &DaemonState) -> Result<()> {
        write_atomic(
            &self.root.join(STATE),
            &serde_json::to_string_pretty(state)?,
        )
    }

    // ---- promoted set ---------------------------------------------------

    pub fn promoted(&self) -> Result<PromotedSet> {
        let path = self.root.join(PROMOTED);
        if !path.exists() {
            return Ok(PromotedSet::default());
        }
        let raw = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    pub fn save_promoted(&self, set: &PromotedSet) -> Result<()> {
        write_atomic(
            &self.root.join(PROMOTED),
            &serde_json::to_string_pretty(set)?,
        )
    }

    fn seed_ask_skillastic(&self) -> Result<()> {
        let mut skill = Skill::new(
            "ask-skillastic",
            Version::new(1, 0, 0),
            vec![],
            Version::new(0, 0, 0),
        );
        skill.invocation = SkillInvocation::UserInvoked;
        skill.bucket = "core".into();
        skill.confidence = 1.0;
        skill.status = SkillStatus::Active;
        self.add_skill(&skill, templates::ASK_SKILLASTIC_BODY)
    }

    // ---- skills ----------------------------------------------------------

    /// Register a new skill with its instruction body.
    /// New skills are always written to the bucket layout.
    pub fn add_skill(&self, skill: &Skill, body: &str) -> Result<()> {
        validate_name(&skill.name)?;
        if self.find_skill_path(&skill.name).is_some() {
            return Err(SkillasticError::SkillExists(skill.name.clone()));
        }
        let mut skill = skill.clone();
        skill.body_path = format!("{}/{}/body.md", skill.bucket, skill.name);
        self.save_skill(&skill)?;
        self.save_body(&skill, body)?;
        Ok(())
    }

    /// Upsert a skill's metadata (does not touch the body).
    /// Preserves the skill's existing on-disk location.
    pub fn save_skill(&self, skill: &Skill) -> Result<()> {
        validate_name(&skill.name)?;
        let path = self
            .find_skill_path(&skill.name)
            .unwrap_or_else(|| self.new_skill_path(skill));
        write_atomic(&path, &serde_json::to_string_pretty(skill)?)
    }

    /// Reflect a resolver decision onto the stored skill's status.
    /// `Load` leaves the status alone; unknown skills are skipped.
    pub fn apply_resolution(&self, res: &Resolution) -> Result<()> {
        let new_status = match res.decision {
            Decision::Validate | Decision::DeepAnalysis => Some(SkillStatus::NeedsValidation),
            Decision::Migrate => Some(SkillStatus::NeedsMigration),
            Decision::Incompatible => Some(SkillStatus::Incompatible),
            Decision::Load => None,
        };
        if let Some(status) = new_status {
            if let Ok(mut skill) = self.load_skill(&res.skill) {
                if skill.status != status {
                    skill.status = status;
                    self.save_skill(&skill)?;
                }
            }
        }
        Ok(())
    }

    pub fn load_skill(&self, name: &str) -> Result<Skill> {
        let path = self
            .find_skill_path(name)
            .ok_or_else(|| SkillasticError::SkillNotFound(name.to_string()))?;
        let raw = fs::read_to_string(path)?;
        let mut skill: Skill = serde_json::from_str(&raw)?;
        // Backward compatibility: legacy skills may be missing the new fields.
        if skill.bucket.is_empty() {
            skill.bucket = "core".into();
        }
        if skill.body_path.is_empty() {
            skill.body_path = format!("{name}.md");
        }
        Ok(skill)
    }

    /// All registered skills, sorted by name.
    pub fn list_skills(&self) -> Result<Vec<Skill>> {
        let mut skills = Vec::new();
        let dir = self.root.join(SKILLS);
        if !dir.is_dir() {
            return Ok(skills);
        }

        // New bucket layout.
        for bucket in BUCKETS {
            let bucket_dir = dir.join(bucket);
            if !bucket_dir.is_dir() {
                continue;
            }
            for entry in fs::read_dir(&bucket_dir)? {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                let meta = entry.path().join("meta.json");
                if meta.is_file() {
                    let raw = fs::read_to_string(&meta)?;
                    skills.push(serde_json::from_str::<Skill>(&raw)?);
                }
            }
        }

        // Legacy flat layout.
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                let raw = fs::read_to_string(&path)?;
                skills.push(serde_json::from_str::<Skill>(&raw)?);
            }
        }

        skills.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(skills)
    }

    pub fn skill_body(&self, skill: &Skill) -> Result<String> {
        Ok(fs::read_to_string(self.body_path(skill))?)
    }

    pub fn save_body(&self, skill: &Skill, body: &str) -> Result<()> {
        write_atomic(&self.body_path(skill), body)
    }

    /// Remove a skill and its body.
    pub fn remove_skill(&self, name: &str) -> Result<()> {
        let path = self
            .find_skill_path(name)
            .ok_or_else(|| SkillasticError::SkillNotFound(name.to_string()))?;
        let skill: Skill = serde_json::from_str(&fs::read_to_string(&path)?)?;
        let body_path = self.body_path(&skill);
        fs::remove_file(&path)?;
        if body_path.exists() {
            fs::remove_file(&body_path)?;
        }
        // Best-effort cleanup of empty bucket skill directory.
        if let Some(parent) = path.parent() {
            let _ = fs::remove_dir(parent);
        }
        Ok(())
    }

    // ---- snapshots --------------------------------------------------------

    /// Freeze the current skill.json + body under `snapshots/<name>@<ver>/`.
    /// Returns the snapshot directory.
    pub fn snapshot(&self, skill: &Skill) -> Result<PathBuf> {
        // Reload from disk so we use the canonical body_path for this workspace layout.
        let skill = self.load_skill(&skill.name)?;
        let dir = self.root.join(SNAPSHOTS).join(skill.id());
        fs::create_dir_all(&dir)?;
        write_atomic(
            &dir.join("skill.json"),
            &serde_json::to_string_pretty(&skill)?,
        )?;
        let body = self.skill_body(&skill).unwrap_or_default();
        write_atomic(&dir.join("body.md"), &body)?;
        Ok(dir)
    }

    /// Load a frozen snapshot by id (`"<name>@<version>"`).
    pub fn load_snapshot(&self, id: &str) -> Result<(Skill, String)> {
        let dir = self.root.join(SNAPSHOTS).join(id);
        if !dir.is_dir() {
            return Err(SkillasticError::Other(format!(
                "no snapshot found for {id}"
            )));
        }
        let skill: Skill = serde_json::from_str(&fs::read_to_string(dir.join("skill.json"))?)?;
        let body = fs::read_to_string(dir.join("body.md")).unwrap_or_default();
        Ok((skill, body))
    }

    /// List snapshot ids for a skill, oldest version first.
    pub fn snapshots_for(&self, name: &str) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        let dir = self.root.join(SNAPSHOTS);
        if dir.is_dir() {
            for entry in fs::read_dir(dir)? {
                let id = entry?.file_name().to_string_lossy().into_owned();
                if id.starts_with(&format!("{name}@")) {
                    ids.push(id);
                }
            }
        }
        ids.sort();
        Ok(ids)
    }

    // ---- bucket READMEs ---------------------------------------------------

    /// Regenerate `.skillastic/skills/<bucket>/README.md` indexes.
    pub fn regenerate_bucket_readmes(&self) -> Result<()> {
        let skills = self.list_skills()?;
        for bucket in BUCKETS {
            let bucket_dir = self.root.join(SKILLS).join(bucket);
            if !bucket_dir.is_dir() {
                continue;
            }
            let mut user_invoked = Vec::new();
            let mut model_invoked = Vec::new();
            let mut both = Vec::new();
            for skill in &skills {
                if skill.bucket == *bucket {
                    match skill.invocation {
                        crate::model::SkillInvocation::UserInvoked => user_invoked.push(skill),
                        crate::model::SkillInvocation::ModelInvoked => model_invoked.push(skill),
                        crate::model::SkillInvocation::Both => both.push(skill),
                    }
                }
            }
            let mut lines = vec![format!("# {bucket} skills"), String::new()];
            if !user_invoked.is_empty() {
                lines.push("## User-invoked".into());
                for skill in &user_invoked {
                    let summary = self
                        .skill_body(skill)
                        .ok()
                        .map(|body| first_line_summary(&body))
                        .unwrap_or_else(|| skill.name.clone());
                    lines.push(format!("- **{}** — {}", skill.name, summary));
                }
                lines.push(String::new());
            }
            if !model_invoked.is_empty() {
                lines.push("## Model-invoked".into());
                for skill in &model_invoked {
                    let summary = self
                        .skill_body(skill)
                        .ok()
                        .map(|body| first_line_summary(&body))
                        .unwrap_or_else(|| skill.name.clone());
                    lines.push(format!("- **{}** — {}", skill.name, summary));
                }
                lines.push(String::new());
            }
            if !both.is_empty() {
                lines.push("## Both".into());
                for skill in &both {
                    let summary = self
                        .skill_body(skill)
                        .ok()
                        .map(|body| first_line_summary(&body))
                        .unwrap_or_else(|| skill.name.clone());
                    lines.push(format!("- **{}** — {}", skill.name, summary));
                }
                lines.push(String::new());
            }
            if user_invoked.is_empty() && model_invoked.is_empty() && both.is_empty() {
                lines.push("No skills in this bucket yet.".into());
                lines.push(String::new());
            }
            write_atomic(&bucket_dir.join("README.md"), &lines.join("\n"))?;
        }
        Ok(())
    }

    // ---- paths ------------------------------------------------------------

    /// Locate an existing skill meta file. Returns `(path, is_legacy)`.
    fn find_skill_path(&self, name: &str) -> Option<PathBuf> {
        // New layout: search all buckets.
        for bucket in BUCKETS {
            let path = self
                .root
                .join(SKILLS)
                .join(bucket)
                .join(name)
                .join("meta.json");
            if path.is_file() {
                return Some(path);
            }
        }
        // Legacy flat layout.
        let legacy = self.root.join(SKILLS).join(format!("{name}.json"));
        if legacy.is_file() {
            return Some(legacy);
        }
        None
    }

    /// Path for a new skill in bucket layout.
    fn new_skill_path(&self, skill: &Skill) -> PathBuf {
        self.root
            .join(SKILLS)
            .join(&skill.bucket)
            .join(&skill.name)
            .join("meta.json")
    }

    fn body_path(&self, skill: &Skill) -> PathBuf {
        self.root.join(SKILLS).join(&skill.body_path)
    }
}

/// Curated set of promoted skill names.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromotedSet {
    pub skills: Vec<String>,
}

fn first_line_summary(body: &str) -> String {
    body.lines()
        .map(|line| line.trim())
        .find(|line| !line.is_empty())
        .map(|line| {
            let line = line.trim_start_matches("#").trim();
            if line.len() > 80 {
                format!("{}…", &line[..80])
            } else {
                line.to_string()
            }
        })
        .unwrap_or_default()
}

/// Skill names become filenames; keep them safe.
fn validate_name(name: &str) -> Result<()> {
    let ok = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !name.starts_with('.');
    if ok {
        Ok(())
    } else {
        Err(SkillasticError::Other(format!(
            "invalid skill name '{name}': use ASCII letters, digits, '-', '_', '.'"
        )))
    }
}

/// Write via a temp file + rename so a crash never leaves a torn file.
fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, contents)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Skill;
    use semver::Version;
    use tempfile::TempDir;

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    fn setup() -> (TempDir, Registry) {
        let dir = TempDir::new().unwrap();
        let registry = Registry::init(dir.path(), Config::default()).unwrap();
        (dir, registry)
    }

    #[test]
    fn init_creates_layout() {
        let (_dir, registry) = setup();
        assert!(registry.root().join(SKILLS).is_dir());
        assert!(registry.root().join(SNAPSHOTS).is_dir());
        assert!(registry.root().join(CONFIG).is_file());
        assert!(registry.root().join(STATE).is_file());
        assert!(registry.root().join(PROMOTED).is_file());
        assert_eq!(registry.config().unwrap().app_name, "app");
    }

    #[test]
    fn double_init_fails() {
        let (dir, _registry) = setup();
        assert!(Registry::init(dir.path(), Config::default()).is_err());
    }

    #[test]
    fn add_load_list_roundtrip() {
        let (_dir, registry) = setup();
        let skill = Skill::new(
            "frontend-react",
            v("2.4.1"),
            vec![">=2.0.0".into()],
            v("2.4.1"),
        );
        registry
            .add_skill(&skill, "# Frontend\nUse Redux.")
            .unwrap();

        assert!(registry.add_skill(&skill, "dup").is_err());

        let loaded = registry.load_skill("frontend-react").unwrap();
        assert_eq!(loaded.skill_version, v("2.4.1"));
        assert_eq!(
            registry.skill_body(&loaded).unwrap(),
            "# Frontend\nUse Redux."
        );
        assert_eq!(loaded.bucket, "core");

        let all = registry.list_skills().unwrap();
        // ask-skillastic is seeded on init, plus the skill we just added.
        assert_eq!(all.len(), 2);
        assert!(registry.load_skill("missing").is_err());
    }

    #[test]
    fn rejects_unsafe_names() {
        let (_dir, registry) = setup();
        for bad in ["../evil", "a/b", "", ".hidden", "sp ace"] {
            let skill = Skill::new(bad, v("1.0.0"), vec![], v("1.0.0"));
            assert!(registry.add_skill(&skill, "x").is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn snapshot_freezes_and_loads() {
        let (_dir, registry) = setup();
        let skill = Skill::new("database", v("1.7.3"), vec![], v("3.2.0"));
        registry.add_skill(&skill, "Use PostgreSQL.").unwrap();

        let dir = registry.snapshot(&skill).unwrap();
        assert!(dir.join("skill.json").is_file());
        assert!(dir.join("body.md").is_file());

        let (frozen, body) = registry.load_snapshot("database@1.7.3").unwrap();
        assert_eq!(frozen.skill_version, v("1.7.3"));
        assert_eq!(body, "Use PostgreSQL.");
        assert_eq!(
            registry.snapshots_for("database").unwrap(),
            vec!["database@1.7.3"]
        );
    }

    #[test]
    fn legacy_flat_layout_still_loads() {
        let (dir, registry) = setup();
        let legacy = dir.path().join(DIR).join(SKILLS).join("legacy.json");
        fs::write(
            &legacy,
            serde_json::json!({
                "name": "legacy",
                "skill_version": "1.0.0",
                "compatible_apps": [],
                "created": "2024-01-01T00:00:00Z",
                "confidence": 0.5,
                "status": "needs_validation",
                "verified_app_version": "1.0.0",
                "body_path": "legacy.md"
            })
            .to_string(),
        )
        .unwrap();
        fs::write(legacy.with_extension("md"), "Legacy body.").unwrap();

        let loaded = registry.load_skill("legacy").unwrap();
        assert_eq!(loaded.bucket, "core");
        assert_eq!(registry.skill_body(&loaded).unwrap(), "Legacy body.");

        let all = registry.list_skills().unwrap();
        // ask-skillastic is seeded on init, plus the legacy skill we created.
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn promoted_set_roundtrip() {
        let (_dir, registry) = setup();
        let mut set = registry.promoted().unwrap();
        // ask-skillastic is seeded on init.
        assert_eq!(set.skills, vec!["ask-skillastic"]);
        set.skills.push("frontend-react".into());
        registry.save_promoted(&set).unwrap();
        let reloaded = registry.promoted().unwrap();
        assert_eq!(reloaded.skills, vec!["ask-skillastic", "frontend-react"]);
    }

    #[test]
    fn bucket_readme_generation() {
        let (_dir, registry) = setup();
        let mut skill = Skill::new("tdd", v("1.0.0"), vec![], v("1.0.0"));
        skill.bucket = "engineering".into();
        skill.invocation = crate::model::SkillInvocation::ModelInvoked;
        registry.add_skill(&skill, "TDD skill body.").unwrap();

        registry.regenerate_bucket_readmes().unwrap();
        let readme = registry
            .root()
            .join(SKILLS)
            .join("engineering")
            .join("README.md");
        assert!(readme.is_file());
        let text = fs::read_to_string(&readme).unwrap();
        assert!(text.contains("## Model-invoked"));
        assert!(text.contains("tdd"));
    }
}
