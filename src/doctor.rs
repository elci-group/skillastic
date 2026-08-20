//! Workspace health checks.

use crate::error::Result;
use crate::registry::Registry;
use std::collections::HashSet;

pub struct Diagnosis {
    pub healthy: bool,
    pub findings: Vec<String>,
}

pub fn diagnose(registry: &Registry) -> Result<Diagnosis> {
    let mut findings = Vec::new();

    // Workspace initialized check is implicit because we hold a Registry.

    let setup_files = [
        "agents/issue-tracker.md",
        "agents/triage-labels.md",
        "agents/domain.md",
    ];
    for file in &setup_files {
        if !registry.root().join(file).is_file() {
            findings.push(format!(
                "missing setup file: .skillastic/{file}; run `skillastic setup`"
            ));
        }
    }

    let skills = registry.list_skills()?;
    let names: HashSet<String> = skills.iter().map(|s| s.name.clone()).collect();

    for skill in &skills {
        let body_path = registry.root().join("skills").join(&skill.body_path);
        if !body_path.is_file() {
            findings.push(format!(
                "skill '{}' is missing its body at {}",
                skill.name,
                body_path.display()
            ));
        }
        for req in &skill.requires {
            if !names.contains(req) {
                findings.push(format!(
                    "skill '{}' requires missing skill '{}'",
                    skill.name, req
                ));
            }
        }
    }

    let promoted = registry.promoted()?;
    for name in &promoted.skills {
        if !names.contains(name) {
            findings.push(format!("promoted skill '{name}' is not registered"));
        }
    }

    let healthy = findings.is_empty();
    Ok(Diagnosis { healthy, findings })
}
