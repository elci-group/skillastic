//! Per-repo setup wizard for Skillastic.
//!
//! Seeds the agent-facing configuration that other commands assume:
//! issue tracker, triage labels, and domain doc layout.

use crate::error::{Result, SkillasticError};
use crate::git::Git;
use crate::registry::Registry;
use crate::templates;
use std::io::{self, Write};
use std::path::Path;

pub struct SetupOptions {
    pub non_interactive: bool,
    pub issue_tracker: Option<String>,
}

pub fn run(registry: &Registry, options: SetupOptions) -> Result<()> {
    let project_root = registry.project_root();
    let agents_dir = registry.root().join("agents");
    std::fs::create_dir_all(&agents_dir)?;

    let provider = if options.non_interactive {
        options
            .issue_tracker
            .clone()
            .unwrap_or_else(|| "github".into())
    } else {
        detect_or_ask_tracker(&project_root)?
    };

    let workflow = tracker_workflow(&provider);
    write_agent_doc(
        &agents_dir.join("issue-tracker.md"),
        &format!(
            "{}\n\n## Provider\n\n{}\n\n## Workflow\n\n{}",
            templates::ISSUE_TRACKER_MD_TEMPLATE
                .lines()
                .next()
                .unwrap_or("# Issue tracker"),
            provider,
            workflow
        ),
    )?;

    write_agent_doc(
        &agents_dir.join("triage-labels.md"),
        templates::TRIAGE_LABELS_MD_TEMPLATE,
    )?;

    write_agent_doc(&agents_dir.join("domain.md"), templates::DOMAIN_MD_TEMPLATE)?;

    // Ensure ADR directory exists.
    std::fs::create_dir_all(registry.root().join("adr"))?;

    Ok(())
}

fn detect_or_ask_tracker(project_root: &Path) -> Result<String> {
    if let Ok(git) = Git::open(project_root) {
        // Best-effort remote detection via git CLI is not exposed here;
        // we fall back to asking.
        let _ = git;
    }

    println!("Issue tracker options:");
    println!("  1. GitHub");
    println!("  2. GitLab");
    println!("  3. Linear");
    println!("  4. Local markdown");
    println!("  5. Other");
    print!("Choose (1-5, default 1): ");
    io::stdout().flush().map_err(SkillasticError::Io)?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(SkillasticError::Io)?;

    let choice = input.trim();
    let provider = match choice {
        "2" => "gitlab",
        "3" => "linear",
        "4" => "local-markdown",
        "5" => "other",
        _ => "github",
    };

    Ok(provider.into())
}

fn tracker_workflow(provider: &str) -> String {
    match provider {
        "github" => "Use `gh issue create` and `gh issue edit`.".into(),
        "gitlab" => "Use `glab issue create` and `glab issue edit`.".into(),
        "linear" => "Use the Linear web UI or CLI to create and update issues.".into(),
        "local-markdown" => "Write one file per issue under `.skillastic/issues/<id>.md`.".into(),
        _ => "Describe your custom workflow here.".into(),
    }
}

fn write_agent_doc(path: &Path, contents: &str) -> Result<()> {
    if path.exists() {
        println!("Preserving existing {}.", path.display());
        return Ok(());
    }
    std::fs::write(path, contents)?;
    Ok(())
}
