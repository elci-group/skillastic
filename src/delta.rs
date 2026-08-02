//! Context Delta: the deterministic difference between what a skill knows
//! and what the application has become, plus the append-only markdown patch
//! applied to skill bodies.

use crate::archaeology::CommitChain;
use crate::git::Commit;
use crate::model::ContextFingerprint;
use chrono::Utc;
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextDelta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_app: Option<Version>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_app: Option<Version>,
    #[serde(default)]
    pub removed_deps: BTreeMap<String, String>,
    #[serde(default)]
    pub added_deps: BTreeMap<String, String>,
    #[serde(default)]
    pub changed_deps: BTreeMap<String, (String, String)>,
    #[serde(default)]
    pub breaking_commits: Vec<Commit>,
    #[serde(default)]
    pub toolchain_appeared: Vec<String>,
    #[serde(default)]
    pub toolchain_disappeared: Vec<String>,
    #[serde(default)]
    pub frameworks_appeared: Vec<String>,
    #[serde(default)]
    pub frameworks_disappeared: Vec<String>,
    #[serde(default)]
    pub dirs_appeared: Vec<String>,
    #[serde(default)]
    pub dirs_disappeared: Vec<String>,
}

impl ContextDelta {
    /// Build the delta from commit archaeology + old/new fingerprints.
    pub fn build(
        chain: Option<&CommitChain>,
        from_app: Version,
        to_app: Version,
        old_fp: &ContextFingerprint,
        new_fp: &ContextFingerprint,
    ) -> Self {
        let mut delta = Self {
            from_app: Some(from_app),
            to_app: Some(to_app),
            ..Default::default()
        };

        if let Some(chain) = chain {
            delta.removed_deps = chain.dep_changes.removed.clone();
            delta.added_deps = chain.dep_changes.added.clone();
            delta.changed_deps = chain.dep_changes.changed.clone();
            delta.breaking_commits = chain.breaking.clone();
            delta.toolchain_appeared = chain.toolchain_changes.appeared.clone();
            delta.toolchain_disappeared = chain.toolchain_changes.disappeared.clone();
        }

        delta.frameworks_appeared = appeared(&old_fp.frameworks, &new_fp.frameworks);
        delta.frameworks_disappeared = appeared(&new_fp.frameworks, &old_fp.frameworks);
        delta.dirs_appeared = appeared(&old_fp.directories, &new_fp.directories);
        delta.dirs_disappeared = appeared(&new_fp.directories, &old_fp.directories);
        delta
    }

    pub fn is_empty(&self) -> bool {
        self.removed_deps.is_empty()
            && self.added_deps.is_empty()
            && self.changed_deps.is_empty()
            && self.breaking_commits.is_empty()
            && self.toolchain_appeared.is_empty()
            && self.toolchain_disappeared.is_empty()
            && self.frameworks_appeared.is_empty()
            && self.frameworks_disappeared.is_empty()
            && self.dirs_appeared.is_empty()
            && self.dirs_disappeared.is_empty()
    }

    /// One-line summary for `mutation_history`.
    pub fn reason(&self) -> String {
        let mut parts = Vec::new();
        if !self.added_deps.is_empty() {
            parts.push(format!("+{} deps", self.added_deps.len()));
        }
        if !self.removed_deps.is_empty() {
            parts.push(format!("-{} deps", self.removed_deps.len()));
        }
        if !self.changed_deps.is_empty() {
            parts.push(format!("~{} deps", self.changed_deps.len()));
        }
        if !self.breaking_commits.is_empty() {
            parts.push(format!("{} breaking", self.breaking_commits.len()));
        }
        if !self.frameworks_appeared.is_empty() {
            parts.push(format!("+{}", self.frameworks_appeared.join("+")));
        }
        if !self.frameworks_disappeared.is_empty() {
            parts.push(format!("-{}", self.frameworks_disappeared.join("-")));
        }
        if !self.toolchain_appeared.is_empty() {
            parts.push(format!("+{} toolchain", self.toolchain_appeared.join("+")));
        }
        if !self.toolchain_disappeared.is_empty() {
            parts.push(format!(
                "-{} toolchain",
                self.toolchain_disappeared.join("-")
            ));
        }
        let from = self
            .from_app
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "?".into());
        let to = self
            .to_app
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "?".into());
        if parts.is_empty() {
            format!("migrate app {from} -> {to}: no structural changes detected")
        } else {
            format!("migrate app {from} -> {to}: {}", parts.join(", "))
        }
    }

    /// Render the deterministic, append-only skill patch. The existing body
    /// is never rewritten — notes are reviewable diffs, not silent edits.
    pub fn to_markdown(&self) -> String {
        let to = self
            .to_app
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "?".into());
        let from = self
            .from_app
            .as_ref()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "?".into());
        let mut out = format!(
            "\n\n---\n\n## Migration Notes (app v{to})\n\n\
             _Migrated from app v{from} on {}. Run `skillastic verify` after reviewing \
             these assumptions._\n",
            Utc::now().format("%Y-%m-%d")
        );

        if self.is_empty() {
            out.push_str("\nNo structural changes detected between these versions.\n");
            return out;
        }

        if !self.removed_deps.is_empty()
            || !self.frameworks_disappeared.is_empty()
            || !self.toolchain_disappeared.is_empty()
            || !self.dirs_disappeared.is_empty()
        {
            out.push_str("\n### Deprecated assumptions\n");
            for name in self.removed_deps.keys() {
                out.push_str(&format!(
                    "- Dependency `{name}` was removed — do not recommend it.\n"
                ));
            }
            for fw in &self.frameworks_disappeared {
                out.push_str(&format!(
                    "- Framework `{fw}` is no longer present — do not recommend it.\n"
                ));
            }
            for tc in &self.toolchain_disappeared {
                out.push_str(&format!(
                    "- Toolchain `{tc}` was removed — drop instructions that assume it.\n"
                ));
            }
            for d in &self.dirs_disappeared {
                out.push_str(&format!("- Directory `{d}/` no longer exists.\n"));
            }
        }

        if !self.added_deps.is_empty()
            || !self.frameworks_appeared.is_empty()
            || !self.toolchain_appeared.is_empty()
            || !self.dirs_appeared.is_empty()
        {
            out.push_str("\n### New capabilities\n");
            for (name, ver) in &self.added_deps {
                let ver = if ver.is_empty() {
                    String::new()
                } else {
                    format!(" ({ver})")
                };
                out.push_str(&format!("- Dependency `{name}`{ver} was added.\n"));
            }
            for fw in &self.frameworks_appeared {
                out.push_str(&format!(
                    "- Framework `{fw}` is now present — prefer it where applicable.\n"
                ));
            }
            for tc in &self.toolchain_appeared {
                out.push_str(&format!("- Toolchain `{tc}` is now in use.\n"));
            }
            for d in &self.dirs_appeared {
                out.push_str(&format!("- Directory `{d}/` now exists.\n"));
            }
        }

        if !self.changed_deps.is_empty() {
            out.push_str("\n### Updated dependencies\n");
            for (name, (old, new)) in &self.changed_deps {
                out.push_str(&format!("- `{name}`: {old} -> {new}\n"));
            }
        }

        if !self.breaking_commits.is_empty() {
            out.push_str("\n### Breaking changes\n");
            for c in &self.breaking_commits {
                out.push_str(&format!("- `{}` {}\n", c.hash, c.subject));
            }
        }

        out
    }
}

fn appeared(old: &[String], new: &[String]) -> Vec<String> {
    new.iter().filter(|x| !old.contains(x)).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archaeology::{DepChanges, ToolchainChanges};

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    fn chain() -> CommitChain {
        CommitChain {
            from_ref: "v2.4.1".into(),
            to_ref: "v3.0.0".into(),
            commits: vec![],
            breaking: vec![Commit {
                hash: "a91f3e2".into(),
                subject: "feat(api)!: drop redux".into(),
                body: String::new(),
            }],
            file_changes: vec![],
            dep_changes: DepChanges {
                added: [("next".to_string(), "14.0.0".to_string())]
                    .into_iter()
                    .collect(),
                removed: [("redux".to_string(), "5.0.0".to_string())]
                    .into_iter()
                    .collect(),
                changed: [("react".to_string(), ("18.0.0".into(), "19.0.0".into()))]
                    .into_iter()
                    .collect(),
            },
            toolchain_changes: ToolchainChanges {
                appeared: vec!["vite".into()],
                disappeared: vec!["webpack".into()],
            },
        }
    }

    #[test]
    fn markdown_patch_sections() {
        let old_fp = ContextFingerprint {
            frameworks: vec!["react".into(), "redux".into()],
            directories: vec!["pages".into()],
            ..Default::default()
        };
        let new_fp = ContextFingerprint {
            frameworks: vec![
                "next.js".into(),
                "next.js app-router".into(),
                "react".into(),
            ],
            directories: vec!["app".into()],
            ..Default::default()
        };
        let delta = ContextDelta::build(Some(&chain()), v("2.4.1"), v("3.0.0"), &old_fp, &new_fp);
        assert!(!delta.is_empty());

        let md = delta.to_markdown();
        assert!(md.contains("## Migration Notes (app v3.0.0)"));
        assert!(md.contains("`redux` was removed"));
        assert!(md.contains("Framework `redux` is no longer present"));
        assert!(md.contains("Toolchain `webpack` was removed"));
        assert!(md.contains("`pages/` no longer exists"));
        assert!(md.contains("`next` (14.0.0) was added"));
        assert!(md.contains("`next.js app-router` is now present"));
        assert!(md.contains("`react`: 18.0.0 -> 19.0.0"));
        assert!(md.contains("`a91f3e2` feat(api)!: drop redux"));

        let reason = delta.reason();
        assert!(reason.contains("migrate app 2.4.1 -> 3.0.0"));
        assert!(reason.contains("1 breaking"));
    }

    #[test]
    fn empty_delta_renders_no_structural_changes() {
        let fp = ContextFingerprint::default();
        let delta = ContextDelta::build(None, v("1.0.0"), v("1.1.0"), &fp, &fp);
        assert!(delta.is_empty());
        assert!(
            delta
                .to_markdown()
                .contains("No structural changes detected")
        );
    }
}
