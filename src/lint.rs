//! Skill and workspace linting.

use crate::domain::{context_terms, find_domain_contradictions};
use crate::error::Result;
use crate::model::SkillInvocation;
use crate::registry::Registry;
use std::collections::HashSet;

pub struct LintReport {
    pub violations: Vec<String>,
}

pub fn lint(registry: &Registry, domain: bool) -> Result<LintReport> {
    let mut violations = Vec::new();
    let skills = registry.list_skills()?;
    let names: HashSet<String> = skills.iter().map(|s| s.name.clone()).collect();
    let promoted = registry.promoted()?;

    for skill in &skills {
        if skill.name.is_empty() {
            violations.push("skill with empty name".into());
            continue;
        }

        // Body must exist.
        let body_path = registry.root().join("skills").join(&skill.body_path);
        if !body_path.is_file() {
            violations.push(format!(
                "{}: missing body at {}",
                skill.name,
                body_path.display()
            ));
        }

        // Invocation/description consistency.
        if let Ok(body) = registry.skill_body(skill) {
            let first_line = body.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
            match skill.invocation {
                SkillInvocation::UserInvoked => {
                    if first_line.to_lowercase().contains("use when") {
                        violations.push(format!(
                            "{}: user-invoked skill should not lead with trigger phrase 'use when'",
                            skill.name
                        ));
                    }
                }
                SkillInvocation::ModelInvoked => {
                    if !first_line.to_lowercase().contains("use when")
                        && !body.to_lowercase().contains("use when")
                    {
                        violations.push(format!(
                            "{}: model-invoked skill should include a 'use when' trigger",
                            skill.name
                        ));
                    }
                }
                SkillInvocation::Both => {}
            }

            // Domain lint.
            if domain {
                let context_path = registry.project_root().join("CONTEXT.md");
                let terms = context_terms(&context_path);
                let contradictions = find_domain_contradictions(&body, &terms);
                for (term, variant) in contradictions {
                    violations.push(format!(
                        "{}: body uses '{}' but CONTEXT.md defines '{}'",
                        skill.name, variant, term
                    ));
                }
            }
        }

        // Dependencies must resolve.
        for req in &skill.requires {
            if !names.contains(req) {
                violations.push(format!("{}: requires missing skill '{}'", skill.name, req));
            }
        }

        // Promoted skills should have a docs page.
        if promoted.skills.contains(&skill.name) {
            let docs_path = registry
                .root()
                .join("docs")
                .join(format!("{}.md", skill.name));
            if !docs_path.is_file() {
                violations.push(format!(
                    "{}: promoted skill missing docs page at {}",
                    skill.name,
                    docs_path.display()
                ));
            }
        }
    }

    // Promoted manifest must point to real skills.
    for name in &promoted.skills {
        if !names.contains(name) {
            violations.push(format!("promoted skill '{name}' is not registered"));
        }
    }

    Ok(LintReport { violations })
}
