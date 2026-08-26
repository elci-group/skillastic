//! Multi-repo monitoring: tracks which project roots should have a
//! `skillastic daemon` running, and starts/checks those daemons.
//!
//! Mirrors the "resume" model used by other repo-watching daemons on this
//! machine: a small JSON registry plus a `resume` operation that spawns a
//! detached daemon for every enabled, not-already-running entry. Designed to
//! be triggered by a login (user scope) or boot (system scope) systemd unit
//! installed via `skillastic enroll`.

use crate::error::{Result, SkillasticError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const IGNORED_DIR_NAMES: &[&str] = &[
    "target",
    "node_modules",
    "dist",
    "build",
    ".next",
    "coverage",
    "__pycache__",
    ".venv",
    "vendor",
];

/// Where a monitor registry (and its systemd unit) lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Per-user: started at login, registry under `$XDG_CONFIG_HOME`.
    User,
    /// Whole-machine: started at boot, registry under `/etc`.
    System,
}

impl Scope {
    pub fn registry_path(self) -> Result<PathBuf> {
        Ok(match self {
            Scope::User => user_config_dir()?.join("skillastic/monitored.json"),
            Scope::System => PathBuf::from("/etc/skillastic/monitored.json"),
        })
    }

    pub fn requires_root(self) -> bool {
        matches!(self, Scope::System)
    }

    pub fn label(self) -> &'static str {
        match self {
            Scope::User => "user",
            Scope::System => "system",
        }
    }
}

fn user_config_dir() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        if !dir.is_empty() {
            return Ok(PathBuf::from(dir));
        }
    }
    let home =
        std::env::var("HOME").map_err(|_| SkillasticError::Other("$HOME is not set".into()))?;
    Ok(PathBuf::from(home).join(".config"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoredProject {
    pub path: PathBuf,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_interval")]
    pub interval: u64,
}

fn default_true() -> bool {
    true
}

fn default_interval() -> u64 {
    300
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MonitorRegistry {
    #[serde(default)]
    pub projects: BTreeMap<String, MonitoredProject>,
}

impl MonitorRegistry {
    pub fn load(scope: Scope) -> Result<Self> {
        Self::load_from(&scope.registry_path()?)
    }

    pub fn save(&self, scope: Scope) -> Result<()> {
        self.save_to(&scope.registry_path()?)
    }

    pub fn load_from(path: &Path) -> Result<Self> {
        if !path.is_file() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&raw)?)
    }

    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }

    pub fn upsert(&mut self, path: &Path, interval: u64) {
        let key = key_for(path);
        self.projects
            .entry(key)
            .and_modify(|p| p.interval = interval)
            .or_insert(MonitoredProject {
                path: path.to_path_buf(),
                enabled: true,
                interval,
            });
    }

    pub fn remove(&mut self, path: &Path) -> bool {
        self.projects.remove(&key_for(path)).is_some()
    }

    pub fn set_enabled(&mut self, path: &Path, enabled: bool) -> Result<()> {
        let key = key_for(path);
        let entry = self
            .projects
            .get_mut(&key)
            .ok_or_else(|| SkillasticError::Other(format!("not monitored: {}", path.display())))?;
        entry.enabled = enabled;
        Ok(())
    }
}

fn key_for(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

/// One repo found while scanning for monitoring candidates.
#[derive(Debug, Clone)]
pub struct DiscoveredRepo {
    pub path: PathBuf,
    pub has_skillastic: bool,
}

/// Finds git repos under `roots`, skipping build/dependency directories and
/// not descending into nested repos (submodules, vendored trees).
pub fn discover_repos(roots: &[PathBuf], max_depth: usize) -> Result<Vec<DiscoveredRepo>> {
    let mut found = Vec::new();
    for root in roots {
        walk(root, 0, max_depth, &mut found);
    }
    found.sort_by(|a: &DiscoveredRepo, b| a.path.cmp(&b.path));
    found.dedup_by(|a, b| a.path == b.path);
    Ok(found)
}

fn walk(dir: &Path, depth: usize, max_depth: usize, found: &mut Vec<DiscoveredRepo>) {
    if depth > max_depth {
        return;
    }
    if dir.join(".git").is_dir() {
        found.push(DiscoveredRepo {
            path: dir.to_path_buf(),
            has_skillastic: dir.join(".skillastic").is_dir(),
        });
        return; // don't descend into nested repos
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return; // unreadable (permissions, races); skip rather than fail the scan
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || IGNORED_DIR_NAMES.contains(&name.as_str()) {
            continue;
        }
        walk(&entry.path(), depth + 1, max_depth, found);
    }
}

/// Outcome of trying to (re)start a monitored project's daemon.
#[derive(Debug, Clone, Serialize)]
pub struct ResumeOutcome {
    pub path: PathBuf,
    pub action: ResumeAction,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResumeAction {
    Started { pid: u32 },
    AlreadyRunning { pid: u32 },
    Skipped { reason: String },
}

impl ResumeOutcome {
    pub fn describe(&self) -> String {
        let path = self.path.display();
        match &self.action {
            ResumeAction::Started { pid } => format!("{path}: started (pid {pid})"),
            ResumeAction::AlreadyRunning { pid } => format!("{path}: already running (pid {pid})"),
            ResumeAction::Skipped { reason } => format!("{path}: skipped ({reason})"),
        }
    }
}

/// Starts daemons for every enabled, not-running entry in `scope`'s registry.
pub fn resume(scope: Scope) -> Result<Vec<ResumeOutcome>> {
    let registry = MonitorRegistry::load(scope)?;
    let mut outcomes = Vec::new();
    for project in registry.projects.values() {
        let action = if !project.enabled {
            ResumeAction::Skipped {
                reason: "disabled".into(),
            }
        } else if !project.path.join(".skillastic").is_dir() {
            ResumeAction::Skipped {
                reason: "no .skillastic workspace".into(),
            }
        } else if let Some(pid) = running_daemon_pid(&project.path) {
            ResumeAction::AlreadyRunning { pid }
        } else {
            ResumeAction::Started {
                pid: spawn_daemon(&project.path, project.interval)?,
            }
        };
        outcomes.push(ResumeOutcome {
            path: project.path.clone(),
            action,
        });
    }
    Ok(outcomes)
}

fn pid_file(path: &Path) -> PathBuf {
    path.join(".skillastic/daemon.pid")
}

/// The daemon's pid for `path`, if its pid file names a live process.
pub fn running_daemon_pid(path: &Path) -> Option<u32> {
    let raw = fs::read_to_string(pid_file(path)).ok()?;
    let pid: u32 = raw.trim().parse().ok()?;
    Path::new(&format!("/proc/{pid}")).is_dir().then_some(pid)
}

fn spawn_daemon(path: &Path, interval: u64) -> Result<u32> {
    let exe = std::env::current_exe()?;
    spawn_detached(&exe, &["daemon", "--interval", &interval.to_string()], path)
}

/// Spawns `exe args...` detached, in `path`, recording its pid. Args are
/// passed through generically so tests can point `exe` at a harmless
/// stand-in (e.g. `/bin/sleep`) instead of re-invoking the real daemon loop.
fn spawn_detached(exe: &Path, args: &[&str], path: &Path) -> Result<u32> {
    let skillastic_dir = path.join(".skillastic");
    fs::create_dir_all(&skillastic_dir)?;
    let out = fs::File::create(skillastic_dir.join("daemon.out"))?;
    let err = fs::File::create(skillastic_dir.join("daemon.err"))?;
    let child = Command::new(exe)
        .args(args)
        .current_dir(path)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .process_group(0) // own process group: survives the parent's shell/session
        .spawn()?;
    let pid = child.id();
    fs::write(pid_file(path), pid.to_string())?;
    // Deliberately not `.wait()`-ed: this is meant to outlive the current
    // process. It gets reparented to init and reaped normally on exit.
    drop(child);
    Ok(pid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_git_repo(root: &Path, rel: &str, with_skillastic: bool) {
        let dir = root.join(rel);
        fs::create_dir_all(dir.join(".git")).unwrap();
        if with_skillastic {
            fs::create_dir_all(dir.join(".skillastic")).unwrap();
        }
    }

    #[test]
    fn discovers_repos_and_flags_skillastic_workspaces() {
        let dir = TempDir::new().unwrap();
        make_git_repo(dir.path(), "alpha", true);
        make_git_repo(dir.path(), "nested/beta", false);
        fs::create_dir_all(dir.path().join("nested/beta/vendor/inner")).unwrap();
        make_git_repo(dir.path(), "nested/beta/vendor/inner", true); // ignored dir, should not appear

        let found = discover_repos(&[dir.path().to_path_buf()], 6).unwrap();
        assert_eq!(found.len(), 2);
        let alpha = found.iter().find(|r| r.path.ends_with("alpha")).unwrap();
        assert!(alpha.has_skillastic);
        let beta = found
            .iter()
            .find(|r| r.path.ends_with("nested/beta"))
            .unwrap();
        assert!(!beta.has_skillastic);
    }

    #[test]
    fn does_not_descend_into_nested_repos() {
        let dir = TempDir::new().unwrap();
        make_git_repo(dir.path(), "outer", false);
        make_git_repo(dir.path(), "outer/inner", true);

        let found = discover_repos(&[dir.path().to_path_buf()], 6).unwrap();
        assert_eq!(found.len(), 1);
        assert!(found[0].path.ends_with("outer"));
    }

    #[test]
    fn registry_round_trips_through_disk() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("monitored.json");
        let mut registry = MonitorRegistry::load_from(&path).unwrap();
        assert!(registry.projects.is_empty());

        registry.upsert(&dir.path().join("proj"), 120);
        registry.save_to(&path).unwrap();

        let reloaded = MonitorRegistry::load_from(&path).unwrap();
        assert_eq!(reloaded.projects.len(), 1);
        let entry = reloaded.projects.values().next().unwrap();
        assert_eq!(entry.interval, 120);
        assert!(entry.enabled);
    }

    #[test]
    fn enable_disable_and_remove_target_the_same_entry() {
        let dir = TempDir::new().unwrap();
        let proj = dir.path().join("proj");
        fs::create_dir_all(&proj).unwrap();
        let mut registry = MonitorRegistry::default();
        registry.upsert(&proj, 60);

        registry.set_enabled(&proj, false).unwrap();
        assert!(!registry.projects.values().next().unwrap().enabled);

        assert!(registry.remove(&proj));
        assert!(registry.projects.is_empty());
    }

    #[test]
    fn set_enabled_on_unknown_path_errors() {
        let dir = TempDir::new().unwrap();
        let mut registry = MonitorRegistry::default();
        assert!(registry.set_enabled(&dir.path().join("nope"), true).is_err());
    }

    #[test]
    fn spawn_daemon_writes_a_live_pid_file() {
        let dir = TempDir::new().unwrap();
        let proj = dir.path().join("proj");
        fs::create_dir_all(&proj).unwrap();
        // Stand in for the real binary with something that just sleeps, so
        // this test never runs actual daemon logic.
        let pid = spawn_detached(Path::new("/bin/sleep"), &["30"], &proj).unwrap();
        assert!(running_daemon_pid(&proj).is_some());
        // Clean up: don't leave a sleeping process behind.
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
}
