//! Thin wrapper over the `git` CLI. No libgit2 — same pattern as kaptaind.

use crate::error::{Result, SkillasticError};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Command;

const FIELD_SEP: char = '\u{1f}';
const RECORD_SEP: char = '\u{1e}';

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commit {
    pub hash: String,
    pub subject: String,
    pub body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub kind: ChangeKind,
    pub path: String,
}

pub struct Git {
    root: PathBuf,
}

impl Git {
    /// Open a repo; errors if `root` is not inside a git work tree.
    pub fn open(root: &Path) -> Result<Self> {
        let git = Self { root: root.to_path_buf() };
        git.run(&["rev-parse", "--is-inside-work-tree"])?;
        Ok(git)
    }

    pub fn is_repo(root: &Path) -> bool {
        Self::open(root).is_ok()
    }

    /// Short SHA of HEAD.
    pub fn head(&self) -> Result<String> {
        Ok(self.run(&["rev-parse", "--short", "HEAD"])?.trim().to_string())
    }

    /// Resolve a version/ref name to a commit-ish.
    /// Tries tags `v<semver>` and `<semver>`, then the name verbatim.
    pub fn resolve_ref(&self, name: &str) -> Option<String> {
        let mut candidates = vec![name.to_string()];
        if let Ok(v) = Version::parse(name) {
            candidates.insert(0, format!("v{v}"));
            candidates.insert(1, v.to_string());
        }
        candidates
            .iter()
            .any(|_| true); // keep clippy quiet about unused mut pattern below
        for cand in &candidates {
            if self.run(&["rev-parse", "--verify", "--quiet", &format!("{cand}^{{commit}}")]).is_ok()
            {
                return Some(cand.clone());
            }
        }
        None
    }

    /// Most recent tag reachable from `to` (excluding `to` itself when
    /// `exclude_self` — used to find the *previous* release tag).
    pub fn latest_tag(&self, to: &str, exclude_self: bool) -> Option<String> {
        let target = if exclude_self { format!("{to}^") } else { to.to_string() };
        self.run(&["describe", "--tags", "--abbrev=0", &target])
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Commits in `from..to`, oldest first.
    pub fn log(&self, from: &str, to: &str) -> Result<Vec<Commit>> {
        let format = format!("%H{FIELD_SEP}%s{FIELD_SEP}%b{RECORD_SEP}");
        let out = self.run(&["log", "--reverse", &format!("--format={format}"), &format!("{from}..{to}")])?;
        let mut commits = Vec::new();
        for record in out.split(RECORD_SEP) {
            let record = record.trim_matches('\n');
            if record.is_empty() {
                continue;
            }
            let mut fields = record.splitn(3, FIELD_SEP);
            let hash = fields.next().unwrap_or_default().trim().to_string();
            let subject = fields.next().unwrap_or_default().trim().to_string();
            let body = fields.next().unwrap_or_default().trim().to_string();
            if !hash.is_empty() {
                commits.push(Commit {
                    hash: hash.chars().take(8).collect(),
                    subject,
                    body,
                });
            }
        }
        Ok(commits)
    }

    /// `git diff --name-status from..to`.
    pub fn diff_name_status(&self, from: &str, to: &str) -> Result<Vec<FileChange>> {
        let out = self.run(&["diff", "--name-status", &format!("{from}..{to}")])?;
        let mut changes = Vec::new();
        for line in out.lines() {
            let mut parts = line.split('\t');
            let (Some(status), Some(path)) = (parts.next(), parts.next()) else {
                continue;
            };
            let kind = match status.chars().next() {
                Some('A') => ChangeKind::Added,
                Some('M') => ChangeKind::Modified,
                Some('D') => ChangeKind::Deleted,
                Some('R') => ChangeKind::Renamed,
                _ => continue,
            };
            // For renames, `path` is the old name and the next field is the new one.
            let path = if kind == ChangeKind::Renamed {
                parts.next().unwrap_or(path).to_string()
            } else {
                path.to_string()
            };
            changes.push(FileChange { kind, path });
        }
        Ok(changes)
    }

    /// File contents at a ref, or None if absent.
    pub fn show_file(&self, ref_: &str, path: &str) -> Option<String> {
        self.run(&["show", &format!("{ref_}:{path}")]).ok()
    }

    pub fn file_exists(&self, ref_: &str, path: &str) -> bool {
        self.run(&["cat-file", "-e", &format!("{ref_}:{path}")]).is_ok()
    }

    /// Top-level entries (files and dirs) of the tree at a ref.
    /// Directory names carry a trailing `/`.
    pub fn ls_tree(&self, ref_: &str) -> Result<Vec<String>> {
        let out = self.run(&["ls-tree", "--name-only", ref_])?;
        Ok(out.lines().map(|l| l.to_string()).collect())
    }

    fn run(&self, args: &[&str]) -> Result<String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .output()?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            Err(SkillasticError::Git(format!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            )))
        }
    }
}
