//! Application version detection.
//!
//! Order: CLI override → `config.json` → `Cargo.toml` → `package.json`
//! → latest git tag.

use crate::error::{Result, SkillasticError};
use crate::git::Git;
use crate::model::Config;
use semver::Version;
use std::fs;
use std::path::Path;

pub fn detect(project_root: &Path, config: &Config, cli_override: Option<&str>) -> Result<Version> {
    if let Some(raw) = cli_override {
        return parse(raw);
    }
    if let Some(v) = &config.app_version {
        return Ok(v.clone());
    }
    if let Some(v) = from_cargo_toml(project_root) {
        return Ok(v);
    }
    if let Some(v) = from_package_json(project_root) {
        return Ok(v);
    }
    if let Ok(git) = Git::open(project_root) {
        if let Some(tag) = git.latest_tag("HEAD", false) {
            if let Ok(v) = parse(&tag) {
                return Ok(v);
            }
        }
    }
    Err(SkillasticError::Other(
        "could not detect the application version; pass --app-version or set app_version in .skillastic/config.json".into(),
    ))
}

/// Parse a semver string, tolerating a leading `v`.
pub fn parse(raw: &str) -> Result<Version> {
    Ok(Version::parse(raw.trim().trim_start_matches('v'))?)
}

fn from_cargo_toml(root: &Path) -> Option<Version> {
    let contents = fs::read_to_string(root.join("Cargo.toml")).ok()?;
    let mut in_package = false;
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if in_package {
            if let Some((key, value)) = line.split_once('=') {
                if key.trim() == "version" {
                    return Version::parse(value.trim().trim_matches('"')).ok();
                }
            }
        }
    }
    None
}

fn from_package_json(root: &Path) -> Option<Version> {
    let raw = fs::read_to_string(root.join("package.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    Version::parse(json.get("version")?.as_str()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn detection_order() {
        let dir = TempDir::new().unwrap();
        let config = Config::default();

        // Nothing yet.
        assert!(detect(dir.path(), &config, None).is_err());
        // CLI override wins even over manifests.
        fs::write(dir.path().join("Cargo.toml"), "[package]\nversion = \"1.2.3\"\n").unwrap();
        assert_eq!(detect(dir.path(), &config, Some("9.9.9")).unwrap(), Version::new(9, 9, 9));
        // Manifest detection.
        assert_eq!(detect(dir.path(), &config, None).unwrap(), Version::new(1, 2, 3));
        // Config pin beats the manifest.
        let pinned = Config { app_version: Some(Version::new(2, 0, 0)), ..Default::default() };
        assert_eq!(detect(dir.path(), &pinned, None).unwrap(), Version::new(2, 0, 0));
    }

    #[test]
    fn tolerates_leading_v() {
        assert_eq!(parse("v3.0.0").unwrap(), Version::new(3, 0, 0));
    }
}
