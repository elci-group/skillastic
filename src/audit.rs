//! Deterministic project-wide audit of Skillastic workspaces.
//!
//! Discovery and workspace health are intentionally local and deterministic.
//! Optional `bound`/Groq enrichment is advisory and never changes the local
//! recommendation.

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const IGNORED_DIRS: &[&str] = &[
    ".git",
    ".skillastic",
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
const MARKERS: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
    "README.md",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditReport {
    pub root: String,
    pub sources: Vec<String>,
    pub projects: Vec<ProjectAudit>,
    pub totals: AuditTotals,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inference: Option<InferenceReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectAudit {
    pub path: String,
    pub name: String,
    pub markers: Vec<String>,
    pub workspace: WorkspaceAudit,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceAudit {
    pub state: WorkspaceState,
    pub skill_count: usize,
    pub valid_skill_count: usize,
    pub invalid_skill_files: Vec<String>,
    pub missing_bodies: Vec<String>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceState {
    Missing,
    Incomplete,
    Empty,
    Ready,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuditTotals {
    pub projects: usize,
    pub missing: usize,
    pub incomplete: usize,
    pub empty: usize,
    pub ready: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceReport {
    pub provider: String,
    pub model: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recommendations: Vec<InferenceRecommendation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceRecommendation {
    pub path: String,
    pub priority: String,
    pub rationale: String,
}

pub fn run(
    root: &Path,
    extra_sources: &[PathBuf],
    infer: bool,
    model: &str,
) -> Result<AuditReport> {
    let mut sources = vec![root.to_path_buf()];
    sources.extend(extra_sources.iter().cloned());
    for name in ["projects", "vico-projects", "work", "src"] {
        let path = root.join(name);
        if path.is_dir() {
            sources.push(path);
        }
    }
    sources.sort();
    sources.dedup();
    let mut project_paths = BTreeSet::new();
    for source in &sources {
        for project in discover_projects(source)? {
            project_paths.insert(project);
        }
    }
    let projects = project_paths
        .into_iter()
        .map(|path| inspect_project(root, &path))
        .collect::<Result<Vec<_>>>()?;
    let totals = projects.iter().fold(AuditTotals::default(), |mut t, p| {
        t.projects += 1;
        match p.workspace.state {
            WorkspaceState::Missing => t.missing += 1,
            WorkspaceState::Incomplete => t.incomplete += 1,
            WorkspaceState::Empty => t.empty += 1,
            WorkspaceState::Ready => t.ready += 1,
        }
        t
    });
    let inference = infer.then(|| infer_recommendations(root, &projects, model));
    Ok(AuditReport {
        root: root.display().to_string(),
        sources: sources.iter().map(|s| s.display().to_string()).collect(),
        projects,
        totals,
        inference,
    })
}

fn discover_projects(root: &Path) -> Result<Vec<PathBuf>> {
    let mut found = BTreeSet::new();
    walk(root, 0, false, true, &mut found)?;
    Ok(found.into_iter().collect())
}

fn walk(
    dir: &Path,
    depth: usize,
    inside_project: bool,
    is_scan_root: bool,
    found: &mut BTreeSet<PathBuf>,
) -> Result<()> {
    if depth > 3 {
        return Ok(());
    }
    let mut markers = Vec::new();
    for marker in MARKERS {
        if dir.join(marker).is_file() {
            markers.push(*marker);
        }
    }
    let git_root = dir.join(".git").is_dir();
    let has_manifest = markers.iter().any(|m| *m != "README.md");
    let has_nested_git = fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .any(|entry| entry.path().join(".git").is_dir());
    if (git_root && !is_scan_root && !(markers.is_empty() && has_nested_git))
        || (!is_scan_root && !inside_project && has_manifest)
    {
        found.insert(dir.to_path_buf());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || IGNORED_DIRS.contains(&name.as_str()) {
            continue;
        }
        walk(
            &entry.path(),
            depth + 1,
            inside_project || git_root,
            false,
            found,
        )?;
    }
    Ok(())
}

fn inspect_project(root: &Path, path: &Path) -> Result<ProjectAudit> {
    let mut markers = MARKERS
        .iter()
        .filter(|m| path.join(m).is_file())
        .map(|m| (*m).into())
        .collect::<Vec<String>>();
    if path.join(".git").is_dir() {
        markers.push(".git".into());
    }
    markers.sort();
    let relative = path.strip_prefix(root).unwrap_or(path);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "root".into());
    Ok(ProjectAudit {
        path: relative.display().to_string(),
        name,
        markers,
        workspace: inspect_workspace(path)?,
    })
}

fn inspect_workspace(project: &Path) -> Result<WorkspaceAudit> {
    let dir = project.join(".skillastic");
    if !dir.is_dir() {
        return Ok(WorkspaceAudit {
            state: WorkspaceState::Missing,
            skill_count: 0,
            valid_skill_count: 0,
            invalid_skill_files: Vec::new(),
            missing_bodies: Vec::new(),
            reasons: vec!["no .skillastic directory".into()],
        });
    }
    let mut reasons = Vec::new();
    for required in ["config.json", "state.json"] {
        if !dir.join(required).is_file() {
            reasons.push(format!("missing .skillastic/{required}"));
        }
    }
    let skills_dir = dir.join("skills");
    let mut skill_count = 0;
    let mut valid_skill_count = 0;
    let mut invalid = Vec::new();
    let mut missing_bodies = Vec::new();
    if skills_dir.is_dir() {
        for entry in fs::read_dir(&skills_dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            skill_count += 1;
            let file = path.file_name().unwrap().to_string_lossy().into_owned();
            let raw = fs::read_to_string(&path)?;
            let parsed = serde_json::from_str::<crate::model::Skill>(&raw);
            let valid = parsed.is_ok();
            if valid {
                valid_skill_count += 1;
            } else {
                invalid.push(file.clone());
            }
            let body = parsed
                .ok()
                .map(|skill| skills_dir.join(skill.body_path))
                .unwrap_or_else(|| path.with_extension("md"));
            if !body.is_file() {
                missing_bodies.push(file.trim_end_matches(".json").to_string());
            }
        }
    } else if skill_count > 0 {
        reasons.push("missing .skillastic/skills directory".into());
    }
    if !invalid.is_empty() {
        reasons.push("invalid skill metadata JSON".into());
    }
    if !missing_bodies.is_empty() {
        reasons.push("skill metadata has no matching Markdown body".into());
    }
    let state = if skill_count == 0 && reasons.is_empty() {
        WorkspaceState::Empty
    } else if reasons.is_empty() {
        WorkspaceState::Ready
    } else {
        WorkspaceState::Incomplete
    };
    Ok(WorkspaceAudit {
        state,
        skill_count,
        valid_skill_count,
        invalid_skill_files: invalid,
        missing_bodies,
        reasons,
    })
}

fn infer_recommendations(root: &Path, projects: &[ProjectAudit], model: &str) -> InferenceReport {
    let mut report = InferenceReport {
        provider: "groq".into(),
        model: model.into(),
        status: "not_run".into(),
        recommendations: Vec::new(),
        error: None,
    };
    let Some(key) = std::env::var_os("GROQ_API_KEY") else {
        report.status = "skipped_no_api_key".into();
        return report;
    };
    let mut bundle = String::new();
    for project in projects
        .iter()
        .filter(|p| p.workspace.state != WorkspaceState::Ready)
    {
        if bundle.len() > 120_000 {
            break;
        }
        bundle.push_str(&format!(
            "PROJECT {} STATE {:?} MARKERS {:?}\n",
            project.path, project.workspace.state, project.markers
        ));
        let path = root.join(&project.path);
        let output = Command::new("bound")
            .args([
                "--json",
                "--meta",
                "--tree",
                "--depth-limit",
                "1",
                "--token-limit",
                "600",
            ])
            .current_dir(&path)
            .output();
        if let Ok(output) = output {
            if output.status.success() {
                bundle.push_str(&String::from_utf8_lossy(&output.stdout));
            }
        }
        bundle.push('\n');
    }
    let prompt = format!(
        "Return JSON array only. Each item must have path, priority (high|medium|low), rationale. Identify which projects should receive a .skillastic workspace next. Do not invent projects. Deterministic audit is authoritative; use this bundle only to prioritize.\n{bundle}"
    );
    let payload = serde_json::json!({"model": model, "temperature": 0, "messages": [{"role": "system", "content": "You audit software project metadata."}, {"role": "user", "content": prompt}]});
    let key = key.to_string_lossy();
    let output = Command::new("curl")
        .args([
            "-sS",
            "https://api.groq.com/openai/v1/chat/completions",
            "-H",
            &format!("Authorization: Bearer {key}"),
            "-H",
            "Content-Type: application/json",
            "--data",
            &payload.to_string(),
        ])
        .output();
    let Ok(output) = output else {
        report.status = "failed_curl".into();
        report.error = Some("could not start curl".into());
        return report;
    };
    if !output.status.success() {
        report.status = "failed_api".into();
        report.error = Some(String::from_utf8_lossy(&output.stderr).trim().to_string());
        return report;
    }
    let response: serde_json::Value = match serde_json::from_slice(&output.stdout) {
        Ok(v) => v,
        Err(e) => {
            report.status = "invalid_api_response".into();
            report.error = Some(e.to_string());
            return report;
        }
    };
    let text = response["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_default()
        .trim();
    let clean = text
        .strip_prefix("```json")
        .unwrap_or(text)
        .strip_suffix("```")
        .unwrap_or(text)
        .trim();
    match serde_json::from_str::<Vec<InferenceRecommendation>>(clean) {
        Ok(items) => {
            report.status = "complete".into();
            report.recommendations = items;
        }
        Err(e) => {
            report.status = "invalid_model_output".into();
            report.error = Some(e.to_string());
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn audit_distinguishes_missing_empty_and_ready() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("README.md"), "root").unwrap();
        fs::write(
            dir.path().join("Cargo.toml"),
            "[package]\nname=\"root\"\nversion=\"0.1.0\"\n",
        )
        .unwrap();
        let missing = dir.path().join("missing");
        fs::create_dir_all(&missing).unwrap();
        fs::write(
            missing.join("Cargo.toml"),
            "[package]\nname=\"missing\"\nversion=\"0.1.0\"\n",
        )
        .unwrap();
        let ready = dir.path().join("ready");
        fs::create_dir_all(ready.join(".skillastic/skills")).unwrap();
        fs::write(ready.join("README.md"), "ready").unwrap();
        fs::write(
            ready.join("Cargo.toml"),
            "[package]\nname=\"ready\"\nversion=\"0.1.0\"\n",
        )
        .unwrap();
        fs::write(ready.join("config.json"), "{}").unwrap_or(());
        fs::write(ready.join(".skillastic/config.json"), "{}").unwrap();
        fs::write(ready.join(".skillastic/state.json"), "{}").unwrap();
        let report = run(dir.path(), &[], false, "test").unwrap();
        assert!(
            report
                .projects
                .iter()
                .any(|p| p.workspace.state == WorkspaceState::Missing)
        );
        assert!(
            report
                .projects
                .iter()
                .any(|p| p.workspace.state == WorkspaceState::Empty)
        );
    }
}
