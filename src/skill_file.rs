//! Validation for loader-facing skill folders (`<name>/SKILL.md`).

use crate::error::Result;
use serde::Serialize;
use serde_yaml::Value;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct Finding {
    pub path: PathBuf,
    pub category: &'static str,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct ValidationReport {
    pub checked: usize,
    pub findings: Vec<Finding>,
}

impl ValidationReport {
    fn finding(&mut self, path: &Path, category: &'static str, message: impl Into<String>) {
        self.findings.push(Finding {
            path: path.to_path_buf(),
            category,
            message: message.into(),
        });
    }
}

/// Validate explicit SKILL.md files, skill folders, or one-level collections.
/// A collection's visible child directories are treated as skill folders.
pub fn validate_paths(paths: &[PathBuf]) -> Result<ValidationReport> {
    let mut report = ValidationReport {
        checked: 0,
        findings: Vec::new(),
    };
    for path in paths {
        if path.is_file() {
            validate_file(path, &mut report)?;
        } else if path.is_dir() {
            if path.join("SKILL.md").is_file() {
                validate_file(&path.join("SKILL.md"), &mut report)?;
            } else {
                let mut children = fs::read_dir(path)?
                    .map(|entry| entry.map(|e| e.path()))
                    .collect::<std::io::Result<Vec<_>>>()?;
                children.sort();
                let mut found = false;
                for child in children {
                    if child.is_dir()
                        && !child
                            .file_name()
                            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
                    {
                        found = true;
                        validate_file(&child.join("SKILL.md"), &mut report)?;
                    }
                }
                if !found {
                    report.finding(path, "shape", "no skill folders found");
                }
            }
        } else {
            report.finding(
                path,
                "shape",
                "path does not exist or is not a file or directory",
            );
        }
    }
    Ok(report)
}

fn validate_file(path: &Path, report: &mut ValidationReport) -> Result<()> {
    report.checked += 1;
    if path.file_name().is_none_or(|name| name != "SKILL.md") {
        report.finding(path, "shape", "skill entrypoint must be named SKILL.md");
        return Ok(());
    }
    if !path.is_file() {
        report.finding(path, "shape", "missing SKILL.md");
        return Ok(());
    }
    let content = fs::read_to_string(path)?;
    let normalized = content.strip_prefix('\u{feff}').unwrap_or(&content);
    let mut lines = normalized.lines();
    if lines.next() != Some("---") {
        report.finding(
            path,
            "shape",
            "frontmatter must start with a standalone --- line",
        );
        return Ok(());
    }
    let mut frontmatter = Vec::new();
    let mut closed = false;
    for line in &mut lines {
        if line == "---" {
            closed = true;
            break;
        }
        frontmatter.push(line);
    }
    if !closed {
        report.finding(
            path,
            "shape",
            "frontmatter needs a closing standalone --- line",
        );
        return Ok(());
    }
    if lines.all(|line| line.trim().is_empty()) {
        report.finding(path, "shape", "skill instructions are empty");
    }
    let yaml = frontmatter.join("\n");
    let value: Value = match serde_yaml::from_str(&yaml) {
        Ok(value) => value,
        Err(error) => {
            report.finding(path, "schema", format!("invalid YAML frontmatter: {error}"));
            return Ok(());
        }
    };
    let Some(mapping) = value.as_mapping() else {
        report.finding(path, "schema", "frontmatter must be a YAML mapping");
        return Ok(());
    };
    let field = |key: &str| mapping.get(Value::String(key.into()));
    for key in mapping.keys() {
        let Some(key) = key.as_str() else {
            report.finding(path, "schema", "frontmatter keys must be strings");
            continue;
        };
        if ![
            "name",
            "description",
            "license",
            "compatibility",
            "allowed-tools",
            "metadata",
        ]
        .contains(&key)
        {
            report.finding(
                path,
                "schema",
                format!("unsupported frontmatter field '{key}'"),
            );
        }
    }
    match field("name") {
        Some(Value::String(name)) => {
            if !valid_name(name) {
                report.finding(
                    path,
                    "schema",
                    "name must use lowercase letters, digits, and single hyphens (1–64 characters)",
                );
            }
            let folder = path.parent().and_then(|parent| parent.canonicalize().ok());
            if folder
                .as_deref()
                .and_then(Path::file_name)
                .is_none_or(|folder| folder != name.as_str())
            {
                report.finding(
                    path,
                    "shape",
                    format!("name '{name}' must match the skill folder"),
                );
            }
        }
        Some(_) => report.finding(path, "schema", "name must be a string"),
        None => report.finding(path, "schema", "missing required name"),
    }
    match field("description") {
        Some(Value::String(description))
            if !description.trim().is_empty() && description.len() <= 1024 => {}
        Some(Value::String(_)) => {
            report.finding(path, "schema", "description must contain 1–1024 characters")
        }
        Some(_) => report.finding(path, "schema", "description must be a string"),
        None => report.finding(path, "schema", "missing required description"),
    }
    for key in ["license", "compatibility", "allowed-tools"] {
        if field(key).is_some_and(|value| !value.is_string()) {
            report.finding(path, "schema", format!("{key} must be a string"));
        }
    }
    if let Some(metadata) = field("metadata") {
        if let Some(entries) = metadata.as_mapping() {
            if entries
                .iter()
                .any(|(key, value)| !key.is_string() || !value.is_string())
            {
                report.finding(path, "schema", "metadata keys and values must be strings");
            }
        } else {
            report.finding(path, "schema", "metadata must be a mapping");
        }
    }
    Ok(())
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn finds_shape_and_schema_failures_in_collection() {
        let dir = TempDir::new().unwrap();
        for name in ["missing", "frontmatter", "yaml", "mismatch", "valid"] {
            fs::create_dir(dir.path().join(name)).unwrap();
        }
        fs::write(dir.path().join("frontmatter/SKILL.md"), "# no header\n").unwrap();
        fs::write(
            dir.path().join("yaml/SKILL.md"),
            "---\nname: yaml\ndescription: >-\nbad indent\n---\n\n# body\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("mismatch/SKILL.md"),
            "---\nname: wrong\ndescription: Here\n---\n\n# body\n",
        )
        .unwrap();
        fs::write(dir.path().join("valid/SKILL.md"), "---\nname: valid\ndescription: >-\n  Helpful skill\nmetadata:\n  author: team\n---\n\n# body\n").unwrap();
        let report = validate_paths(&[dir.path().to_path_buf()]).unwrap();
        assert_eq!(report.checked, 5);
        assert_eq!(report.findings.len(), 4);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.message.contains("missing SKILL.md"))
        );
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.message.contains("invalid YAML"))
        );
    }

    #[test]
    fn rejects_wrong_types_and_empty_body() {
        let dir = TempDir::new().unwrap();
        let skill = dir.path().join("sample");
        fs::create_dir(&skill).unwrap();
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: 2\ndescription: []\nmetadata: nope\n---\n",
        )
        .unwrap();
        let report = validate_paths(&[skill]).unwrap();
        assert_eq!(report.findings.len(), 4);
    }

    #[test]
    fn rejects_unknown_fields_and_non_string_metadata_values() {
        let dir = TempDir::new().unwrap();
        let skill = dir.path().join("sample");
        fs::create_dir(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "---\nname: sample\ndescription: Works\nunknown: true\nmetadata:\n  version: 2\n---\n\n# Body\n").unwrap();
        let report = validate_paths(&[skill]).unwrap();
        assert_eq!(report.findings.len(), 2);
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.message.contains("unsupported"))
        );
        assert!(
            report
                .findings
                .iter()
                .any(|f| f.message.contains("metadata keys"))
        );
    }
}
