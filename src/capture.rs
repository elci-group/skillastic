//! Context Capture Engine: fingerprints what a codebase actually is —
//! frameworks, toolchains, directory shape, dependencies — either from the
//! working tree or reconstructed at a git ref.

use crate::archaeology::{TOOLCHAIN_FILES, parse_manifest};
use crate::error::Result;
use crate::git::Git;
use crate::model::ContextFingerprint;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Manifests scanned for dependencies (mirrors archaeology).
const MANIFESTS: &[&str] = &["package.json", "Cargo.toml", "requirements.txt"];

/// Directories that carry no architectural signal.
const NOISE_DIRS: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    ".skillastic",
    "dist",
    "build",
    "out",
    ".next",
    "coverage",
    "__pycache__",
    ".venv",
];

/// Dependency name → framework. Architecturally meaningful only —
/// low-level utilities (serde, lodash) are deliberately excluded.
const FRAMEWORK_DEPS: &[(&str, &str)] = &[
    ("next", "next.js"),
    ("react", "react"),
    ("@reduxjs/toolkit", "redux"),
    ("redux", "redux"),
    ("zustand", "zustand"),
    ("vue", "vue"),
    ("nuxt", "nuxt"),
    ("svelte", "svelte"),
    ("@angular/core", "angular"),
    ("express", "express"),
    ("fastify", "fastify"),
    ("tailwindcss", "tailwindcss"),
    ("tokio", "tokio"),
    ("axum", "axum"),
    ("actix-web", "actix-web"),
    ("sqlx", "sqlx"),
    ("diesel", "diesel"),
    ("ratatui", "ratatui"),
    ("django", "django"),
    ("flask", "flask"),
    ("fastapi", "fastapi"),
    ("sqlalchemy", "sqlalchemy"),
];

pub struct Capture;

impl Capture {
    /// Fingerprint the current working tree.
    pub fn scan(project_root: &Path) -> Result<ContextFingerprint> {
        let mut directories = Vec::new();
        for entry in fs::read_dir(project_root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with('.') && !NOISE_DIRS.contains(&name.as_str()) {
                directories.push(name);
            }
        }

        let mut dependencies = BTreeMap::new();
        for manifest in MANIFESTS {
            let path = project_root.join(manifest);
            if path.is_file() {
                dependencies.extend(parse_manifest(manifest, &fs::read_to_string(path)?));
            }
        }

        let toolchains: Vec<String> = TOOLCHAIN_FILES
            .iter()
            .filter(|(file, _)| project_root.join(file).is_file())
            .map(|(_, name)| name.to_string())
            .collect();

        Ok(assemble(directories, dependencies, toolchains))
    }

    /// Reconstruct the fingerprint at a git ref (for old-context diffs).
    pub fn scan_ref(git: &Git, ref_: &str) -> Result<ContextFingerprint> {
        let directories: Vec<String> = git
            .ls_tree(ref_)?
            .into_iter()
            .filter_map(|name| {
                // ls_tree marks dirs with a trailing '/' when using --name-only
                // only with -d; probe entries ending in '/' or known by type.
                let name = name.trim_end_matches('/').to_string();
                if git_dir(git, ref_, &name)
                    && !name.starts_with('.')
                    && !NOISE_DIRS.contains(&name.as_str())
                {
                    Some(name)
                } else {
                    None
                }
            })
            .collect();

        let mut dependencies = BTreeMap::new();
        for manifest in MANIFESTS {
            if let Some(raw) = git.show_file(ref_, manifest) {
                dependencies.extend(parse_manifest(manifest, &raw));
            }
        }

        let toolchains: Vec<String> = TOOLCHAIN_FILES
            .iter()
            .filter(|(file, _)| git.file_exists(ref_, file))
            .map(|(_, name)| name.to_string())
            .collect();

        Ok(assemble(directories, dependencies, toolchains))
    }
}

/// Is `name` a directory at `ref_`? (ls-tree --name-only doesn't mark types.)
fn git_dir(git: &Git, ref_: &str, name: &str) -> bool {
    git.object_type(ref_, name).as_deref() == Some("tree")
}

fn assemble(
    mut directories: Vec<String>,
    dependencies: BTreeMap<String, String>,
    toolchains: Vec<String>,
) -> ContextFingerprint {
    directories.sort();
    directories.dedup();

    let mut frameworks: Vec<String> = FRAMEWORK_DEPS
        .iter()
        .filter(|(dep, _)| dependencies.contains_key(*dep))
        .map(|(_, fw)| fw.to_string())
        .collect();

    // Router flavour detection for next.js.
    if dependencies.contains_key("next") {
        if directories.iter().any(|d| d == "app") {
            frameworks.push("next.js app-router".into());
        }
        if directories.iter().any(|d| d == "pages") {
            frameworks.push("next.js pages-router".into());
        }
    }

    frameworks.sort();
    frameworks.dedup();

    let mut toolchains = toolchains;
    toolchains.sort();
    toolchains.dedup();

    ContextFingerprint {
        frameworks,
        toolchains,
        directories,
        dependencies,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn scan_infers_frameworks_and_toolchains() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::write(
            root.join("package.json"),
            r#"{"dependencies": {"next": "14.0.0", "react": "^18.0.0", "zustand": "^4.0.0"}}"#,
        )
        .unwrap();
        fs::create_dir(root.join("app")).unwrap();
        fs::create_dir(root.join("components")).unwrap();
        fs::create_dir(root.join("node_modules")).unwrap(); // noise, excluded
        fs::write(root.join("tsconfig.json"), "{}").unwrap();
        fs::write(root.join("next.config.mjs"), "export default {}").unwrap();

        let fp = Capture::scan(root).unwrap();
        assert!(fp.frameworks.contains(&"next.js".to_string()));
        assert!(fp.frameworks.contains(&"next.js app-router".to_string()));
        assert!(fp.frameworks.contains(&"react".to_string()));
        assert!(fp.frameworks.contains(&"zustand".to_string()));
        assert!(!fp.frameworks.contains(&"redux".to_string()));
        assert!(fp.toolchains.contains(&"typescript".to_string()));
        assert!(fp.toolchains.contains(&"next.js".to_string()));
        assert!(fp.directories.contains(&"app".to_string()));
        assert!(!fp.directories.contains(&"node_modules".to_string()));
        assert_eq!(fp.dependencies.get("next").unwrap(), "14.0.0");
    }

    #[test]
    fn empty_project_yields_empty_fingerprint() {
        let dir = TempDir::new().unwrap();
        let fp = Capture::scan(dir.path()).unwrap();
        assert!(fp.is_empty());
    }
}
