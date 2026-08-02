//! Skill Registry: the on-disk `.skillastic/` workspace.
//!
//! Layout:
//! ```text
//! .skillastic/
//!   config.json                 workspace config
//!   state.json                  daemon state
//!   skills/<name>.json          skill object (metadata + lineage)
//!   skills/<name>.md            instruction body
//!   snapshots/<name>@<ver>/     frozen skill.json + body.md per migration
//! ```

use crate::error::{Result, SkillasticError};
use crate::model::{Config, DaemonState, Decision, Resolution, Skill, SkillStatus};
use std::fs;
use std::path::{Path, PathBuf};

pub const DIR: &str = ".skillastic";
const SKILLS: &str = "skills";
const SNAPSHOTS: &str = "snapshots";
const CONFIG: &str = "config.json";
const STATE: &str = "state.json";

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
        fs::create_dir_all(root.join(SNAPSHOTS))?;
        let registry = Self { root };
        registry.save_config(&config)?;
        registry.save_state(&DaemonState::default())?;
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

    // ---- skills ----------------------------------------------------------

    /// Register a new skill with its instruction body.
    pub fn add_skill(&self, skill: &Skill, body: &str) -> Result<()> {
        validate_name(&skill.name)?;
        if self.skill_path(&skill.name).exists() {
            return Err(SkillasticError::SkillExists(skill.name.clone()));
        }
        self.save_skill(skill)?;
        self.save_body(skill, body)?;
        Ok(())
    }

    /// Upsert a skill's metadata (does not touch the body).
    pub fn save_skill(&self, skill: &Skill) -> Result<()> {
        validate_name(&skill.name)?;
        write_atomic(
            &self.skill_path(&skill.name),
            &serde_json::to_string_pretty(skill)?,
        )
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
        let path = self.skill_path(name);
        if !path.exists() {
            return Err(SkillasticError::SkillNotFound(name.to_string()));
        }
        let raw = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    /// All registered skills, sorted by name.
    pub fn list_skills(&self) -> Result<Vec<Skill>> {
        let mut skills = Vec::new();
        let dir = self.root.join(SKILLS);
        if !dir.is_dir() {
            return Ok(skills);
        }
        for entry in fs::read_dir(dir)? {
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

    // ---- snapshots --------------------------------------------------------

    /// Freeze the current skill.json + body under `snapshots/<name>@<ver>/`.
    /// Returns the snapshot directory.
    pub fn snapshot(&self, skill: &Skill) -> Result<PathBuf> {
        let dir = self.root.join(SNAPSHOTS).join(skill.id());
        fs::create_dir_all(&dir)?;
        write_atomic(
            &dir.join("skill.json"),
            &serde_json::to_string_pretty(skill)?,
        )?;
        let body = self.skill_body(skill).unwrap_or_default();
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

    // ---- paths ------------------------------------------------------------

    fn skill_path(&self, name: &str) -> PathBuf {
        self.root.join(SKILLS).join(format!("{name}.json"))
    }

    fn body_path(&self, skill: &Skill) -> PathBuf {
        self.root.join(SKILLS).join(&skill.body_path)
    }
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

        let all = registry.list_skills().unwrap();
        assert_eq!(all.len(), 1);
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
}
