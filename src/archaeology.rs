//! Commit Archaeology Engine: reconstructs how the application evolved
//! between two refs — commits, breaking markers, file churn, dependency
//! movement, toolchain changes.

use crate::error::{Result, SkillasticError};
use crate::git::{Commit, FileChange, Git};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Dependency movement between two refs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DepChanges {
    pub added: BTreeMap<String, String>,
    pub removed: BTreeMap<String, String>,
    /// name → (old req, new req)
    pub changed: BTreeMap<String, (String, String)>,
}

impl DepChanges {
    pub fn any(&self) -> bool {
        !self.added.is_empty() || !self.removed.is_empty() || !self.changed.is_empty()
    }

    pub fn is_empty(&self) -> bool {
        !self.any()
    }
}

/// Toolchain marker files that appeared/disappeared between refs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolchainChanges {
    pub appeared: Vec<String>,
    pub disappeared: Vec<String>,
}

/// The full evolution record between two refs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitChain {
    pub from_ref: String,
    pub to_ref: String,
    pub commits: Vec<Commit>,
    /// Commits carrying a breaking-change marker.
    pub breaking: Vec<Commit>,
    pub file_changes: Vec<FileChange>,
    pub dep_changes: DepChanges,
    pub toolchain_changes: ToolchainChanges,
}

/// Manifests we know how to diff for dependency movement.
const MANIFESTS: &[&str] = &["package.json", "Cargo.toml", "requirements.txt"];

/// Toolchain marker file → human name. Probed at each ref.
const TOOLCHAIN_FILES: &[(&str, &str)] = &[
    ("webpack.config.js", "webpack"),
    ("webpack.config.ts", "webpack"),
    ("vite.config.js", "vite"),
    ("vite.config.ts", "vite"),
    ("vite.config.mjs", "vite"),
    ("next.config.js", "next.js"),
    ("next.config.mjs", "next.js"),
    ("next.config.ts", "next.js"),
    ("tsconfig.json", "typescript"),
    ("Dockerfile", "docker"),
    ("docker-compose.yml", "docker-compose"),
    ("compose.yml", "docker-compose"),
    ("tailwind.config.js", "tailwind"),
    ("tailwind.config.ts", "tailwind"),
    ("babel.config.js", "babel"),
    (".babelrc", "babel"),
    ("jest.config.js", "jest"),
    ("jest.config.ts", "jest"),
    ("vitest.config.ts", "vitest"),
    ("eslint.config.js", "eslint"),
    (".eslintrc.json", "eslint"),
    ("rust-toolchain.toml", "rust-toolchain"),
];

pub struct Archaeology {
    git: Git,
}

impl Archaeology {
    pub fn new(project_root: &Path) -> Result<Self> {
        Ok(Self { git: Git::open(project_root)? })
    }

    pub fn git(&self) -> &Git {
        &self.git
    }

    /// Analyze the chain between two version/ref names.
    pub fn analyze(&self, from: &str, to: &str) -> Result<CommitChain> {
        let from_ref = self
            .git
            .resolve_ref(from)
            .ok_or_else(|| SkillasticError::Git(format!("cannot resolve ref '{from}'")))?;
        let to_ref = self
            .git
            .resolve_ref(to)
            .ok_or_else(|| SkillasticError::Git(format!("cannot resolve ref '{to}'")))?;
        self.analyze_refs(&from_ref, &to_ref)
    }

    /// Analyze the chain between two already-resolved refs.
    pub fn analyze_refs(&self, from_ref: &str, to_ref: &str) -> Result<CommitChain> {
        let commits = self.git.log(from_ref, to_ref)?;
        let breaking = commits.iter().filter(|c| is_breaking(c)).cloned().collect();
        let file_changes = self.git.diff_name_status(from_ref, to_ref)?;
        let dep_changes = self.dep_changes(from_ref, to_ref);
        let toolchain_changes = self.toolchain_changes(from_ref, to_ref);
        Ok(CommitChain {
            from_ref: from_ref.to_string(),
            to_ref: to_ref.to_string(),
            commits,
            breaking,
            file_changes,
            dep_changes,
            toolchain_changes,
        })
    }

    /// Did dependencies move between two refs? Used by the resolver for
    /// the "unknown dependency change" rule.
    pub fn deps_changed(&self, from_ref: &str, to_ref: &str) -> bool {
        self.dep_changes(from_ref, to_ref).any()
    }

    /// Default starting point for an analysis: the tag before `to`.
    pub fn previous_tag(&self, to: &str) -> Option<String> {
        self.git.latest_tag(to, true)
    }

    fn dep_changes(&self, from_ref: &str, to_ref: &str) -> DepChanges {
        let mut old = BTreeMap::new();
        let mut new = BTreeMap::new();
        for manifest in MANIFESTS {
            if let Some(raw) = self.git.show_file(from_ref, manifest) {
                old.extend(parse_manifest(manifest, &raw));
            }
            if let Some(raw) = self.git.show_file(to_ref, manifest) {
                new.extend(parse_manifest(manifest, &raw));
            }
        }
        diff_deps(&old, &new)
    }

    fn toolchain_changes(&self, from_ref: &str, to_ref: &str) -> ToolchainChanges {
        let at = |ref_: &str| -> Vec<&str> {
            TOOLCHAIN_FILES
                .iter()
                .filter(|(file, _)| self.git.file_exists(ref_, file))
                .map(|(_, name)| *name)
                .collect()
        };
        let before = at(from_ref);
        let after = at(to_ref);
        ToolchainChanges {
            appeared: after.iter().filter(|t| !before.contains(t)).map(|s| s.to_string()).collect(),
            disappeared: before.iter().filter(|t| !after.contains(t)).map(|s| s.to_string()).collect(),
        }
    }
}

/// Conventional-commit and trailer based breaking-change detection.
pub fn is_breaking(commit: &Commit) -> bool {
    let subject_breaks = {
        // `type(scope)!:` or `type!:` — '!' immediately before the colon.
        let head = commit.subject.split(':').next().unwrap_or_default();
        head.ends_with('!') && commit.subject.contains(':')
    };
    subject_breaks
        || commit.body.contains("BREAKING CHANGE")
        || commit.body.contains("BREAKING-CHANGE")
}

/// Parse a manifest into (name → version req) pairs.
pub fn parse_manifest(filename: &str, contents: &str) -> BTreeMap<String, String> {
    match filename {
        "package.json" => parse_package_json(contents),
        "Cargo.toml" => parse_cargo_toml(contents),
        "requirements.txt" => parse_requirements(contents),
        _ => BTreeMap::new(),
    }
}

fn parse_package_json(contents: &str) -> BTreeMap<String, String> {
    let Ok(json) = serde_json::from_str::<serde_json::Value>(contents) else {
        return BTreeMap::new();
    };
    let mut deps = BTreeMap::new();
    for section in ["dependencies", "devDependencies"] {
        if let Some(map) = json.get(section).and_then(|s| s.as_object()) {
            for (name, ver) in map {
                deps.insert(name.clone(), ver.as_str().unwrap_or_default().to_string());
            }
        }
    }
    deps
}

/// Minimal line parser covering the common `[dependencies]` shapes:
/// `name = "ver"` and `name = { version = "ver", ... }`.
fn parse_cargo_toml(contents: &str) -> BTreeMap<String, String> {
    let mut deps = BTreeMap::new();
    let mut in_deps = false;
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_deps = line.trim_matches(['[', ']']).ends_with("dependencies");
            continue;
        }
        if !in_deps || line.starts_with('#') {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else { continue };
        let name = name.trim().trim_matches('"');
        if name.is_empty() {
            continue;
        }
        let value = value.trim();
        let version = if let Some(quoted) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
            quoted.to_string()
        } else if value.starts_with('{') {
            // Inline table: find `version = "..."`.
            value
                .trim_start_matches('{')
                .trim_end_matches('}')
                .split(',')
                .filter_map(|kv| kv.split_once('='))
                .find(|(k, _)| k.trim() == "version")
                .map(|(_, v)| v.trim().trim_matches(|c| c == '"' || c == '}').to_string())
                .unwrap_or_default()
        } else {
            continue;
        };
        deps.insert(name.to_string(), version);
    }
    deps
}

fn parse_requirements(contents: &str) -> BTreeMap<String, String> {
    let mut deps = BTreeMap::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('-') {
            continue;
        }
        if let Some((name, ver)) = line.split_once("==") {
            deps.insert(name.trim().to_string(), ver.trim().to_string());
        } else {
            deps.insert(line.to_string(), String::new());
        }
    }
    deps
}

fn diff_deps(
    old: &BTreeMap<String, String>,
    new: &BTreeMap<String, String>,
) -> DepChanges {
    let mut changes = DepChanges::default();
    for (name, ver) in new {
        match old.get(name) {
            None => {
                changes.added.insert(name.clone(), ver.clone());
            }
            Some(old_ver) if old_ver != ver => {
                changes.changed.insert(name.clone(), (old_ver.clone(), ver.clone()));
            }
            _ => {}
        }
    }
    for (name, ver) in old {
        if !new.contains_key(name) {
            changes.removed.insert(name.clone(), ver.clone());
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breaking_markers() {
        let breaking_subject = Commit {
            hash: "a".into(),
            subject: "feat(api)!: drop v1 endpoints".into(),
            body: String::new(),
        };
        let breaking_body = Commit {
            hash: "b".into(),
            subject: "feat: rework config".into(),
            body: "Some text\n\nBREAKING CHANGE: config file moved".into(),
        };
        let normal = Commit {
            hash: "c".into(),
            subject: "fix: typo!".into(),
            body: String::new(),
        };
        assert!(is_breaking(&breaking_subject));
        assert!(is_breaking(&breaking_body));
        assert!(!is_breaking(&normal));
    }

    #[test]
    fn package_json_parsing() {
        let raw = r#"{
            "dependencies": { "react": "^18.2.0", "next": "14.0.0" },
            "devDependencies": { "vitest": "^1.0.0" }
        }"#;
        let deps = parse_manifest("package.json", raw);
        assert_eq!(deps.get("react").unwrap(), "^18.2.0");
        assert_eq!(deps.get("next").unwrap(), "14.0.0");
        assert_eq!(deps.get("vitest").unwrap(), "^1.0.0");
    }

    #[test]
    fn cargo_toml_parsing() {
        let raw = r#"
[package]
name = "x"

[dependencies]
serde = { version = "1", features = ["derive"] }
semver = "1"
# comment = "ignored"

[dev-dependencies]
tempfile = "3"
"#;
        let deps = parse_manifest("Cargo.toml", raw);
        assert_eq!(deps.get("serde").unwrap(), "1");
        assert_eq!(deps.get("semver").unwrap(), "1");
        assert_eq!(deps.get("tempfile").unwrap(), "3");
        assert!(!deps.contains_key("name")); // [package] section ignored
    }

    #[test]
    fn dep_diff() {
        let old: BTreeMap<_, _> =
            [("redux", "5.0"), ("react", "18.0")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let new: BTreeMap<_, _> =
            [("react", "19.0"), ("zustand", "4.5")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let diff = diff_deps(&old, &new);
        assert!(diff.added.contains_key("zustand"));
        assert!(diff.removed.contains_key("redux"));
        assert_eq!(diff.changed.get("react").unwrap(), &("18.0".into(), "19.0".into()));
        assert!(diff.any());
    }
}
