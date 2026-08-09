use clap::{Parser, Subcommand};
use serde::Serialize;
use skillastic::SkillasticError;
use skillastic::appver;
use skillastic::archaeology::Archaeology;
use skillastic::audit;
use skillastic::capture::Capture;
use skillastic::daemon::{Daemon, recent_events};
use skillastic::error::Result;
use skillastic::migrate::{Migrator, verify};
use skillastic::model::{Config, Decision, Skill};
use skillastic::registry::Registry;
use skillastic::resolver::{Resolver, parse_req};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Skillastic — adaptive skill runtime.
///
/// Keeps AI-agent skills compatible with the application they describe:
/// deterministic version resolution, commit archaeology, context capture,
/// and skill migration with lineage.
#[derive(Parser)]
#[command(name = "skillastic", version, about)]
struct Cli {
    /// Emit machine-readable JSON.
    #[arg(long, global = true)]
    json: bool,

    /// Override the application version (semver) instead of auto-detecting.
    #[arg(long, global = true, value_name = "SEMVER")]
    app_version: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Initialize a .skillastic workspace in the current project.
    Init {
        /// Application name (defaults to the directory name).
        #[arg(long)]
        app_name: Option<String>,
    },

    /// Register a new skill.
    Add {
        /// Skill name, e.g. "frontend-react".
        name: String,
        /// Initial skill version.
        #[arg(long)]
        version: String,
        /// Compatible app range(s), e.g. ">=2.1.0, <3.0.0". Repeatable.
        #[arg(long, required = true)]
        compatible: Vec<String>,
        /// Markdown file with the skill's instruction body.
        #[arg(long)]
        body: Option<PathBuf>,
        /// Mark the skill verified against the current app version.
        #[arg(long)]
        verify: bool,
    },

    /// List registered skills.
    List,

    /// Show one skill in full.
    Show {
        /// Skill name.
        name: String,
    },

    /// Version-resolver report for all skills vs. the current app version.
    Status,

    /// Commit-chain analysis between two app versions.
    Archaeology {
        /// Start version/ref (default: tag before --to).
        #[arg(long)]
        from: Option<String>,
        /// End version/ref (default: HEAD).
        #[arg(long)]
        to: Option<String>,
    },

    /// Capture the current codebase context fingerprint.
    Capture,

    /// Migrate a skill (or --all) to the current app version.
    Migrate {
        /// Skill name; omit with --all.
        name: Option<String>,
        /// Migrate every skill the resolver marks `migrate`.
        #[arg(long)]
        all: bool,
        /// Compute and print the delta without writing anything.
        #[arg(long)]
        dry_run: bool,
    },

    /// Mark a skill verified against the current app version.
    Verify {
        /// Skill name.
        name: String,
    },

    /// Show a skill's lineage chain and mutation history.
    History {
        /// Skill name.
        name: String,
    },

    /// Run the skillastic daemon (poll for app changes, auto-resolve/migrate).
    Daemon {
        /// Poll interval in seconds.
        #[arg(long, default_value_t = 60)]
        interval: u64,
    },

    /// Show recent daemon events recorded in the workspace state.
    Events,

    /// Audit project roots for missing or incomplete .skillastic workspaces.
    Audit {
        /// Directory to scan (defaults to the current directory).
        #[arg(long, default_value = ".")]
        root: PathBuf,
        /// Additional project-source directory; repeat for multiple sources.
        #[arg(long = "source")]
        sources: Vec<PathBuf>,
        /// Ask Groq to prioritize deterministic findings using bound context.
        #[arg(long)]
        infer: bool,
        /// Groq model used for advisory prioritization.
        #[arg(long, default_value = "llama-3.1-8b-instant")]
        model: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let project_root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    match run(cli, &project_root) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli, project_root: &Path) -> Result<()> {
    let json = cli.json;
    let app_version_override = cli.app_version.as_deref();

    match cli.command {
        Command::Init { app_name } => {
            let name = app_name.unwrap_or_else(|| {
                project_root
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "app".into())
            });
            let pinned = match app_version_override {
                Some(raw) => Some(appver::parse(raw)?),
                None => None,
            };
            let config = Config {
                app_name: name.clone(),
                app_version: pinned,
                ..Default::default()
            };
            let registry = Registry::init(project_root, config)?;
            if json {
                print_json(&serde_json::json!({
                    "initialized": registry.root(),
                    "app_name": name,
                }));
            } else {
                println!(
                    "Initialized skillastic workspace for '{name}' in {}",
                    registry.root().display()
                );
            }
        }

        Command::Add {
            name,
            version,
            compatible,
            body,
            verify: verify_now,
        } => {
            let registry = open_registry(project_root)?;
            let config = registry.config()?;
            let app_version = appver::detect(project_root, &config, app_version_override)?;
            for range in &compatible {
                parse_req(range)?; // fail fast on garbage ranges
            }
            let mut skill = Skill::new(
                &name,
                appver::parse(&version)?,
                compatible,
                app_version.clone(),
            );
            skill.context = Capture::scan(project_root)?;
            let body = match body {
                Some(path) => std::fs::read_to_string(path)?,
                None => format!("# {name}\n\nDescribe how to work with this subsystem.\n"),
            };
            registry.add_skill(&skill, &body)?;
            if verify_now {
                skill = verify(&registry, &name, &app_version)?;
            }
            if json {
                print_json(&skill);
            } else {
                println!(
                    "Registered skill '{}' v{} against app {app_version} (status: {})",
                    skill.name, skill.skill_version, skill.status
                );
            }
        }

        Command::List => {
            let registry = open_registry(project_root)?;
            let skills = registry.list_skills()?;
            if json {
                print_json(&skills);
            } else if skills.is_empty() {
                println!("No skills registered. Use `skillastic add`.");
            } else {
                let rows = skills
                    .iter()
                    .map(|s| {
                        vec![
                            s.name.clone(),
                            s.skill_version.to_string(),
                            s.status.to_string(),
                            format!("{:.2}", s.confidence),
                            s.compatible_apps.join(" | "),
                            s.verified_app_version.to_string(),
                        ]
                    })
                    .collect();
                print!(
                    "{}",
                    table(
                        &[
                            "SKILL",
                            "VERSION",
                            "STATUS",
                            "CONF",
                            "COMPATIBLE APPS",
                            "VERIFIED APP"
                        ],
                        rows,
                    )
                );
            }
        }

        Command::Show { name } => {
            let registry = open_registry(project_root)?;
            let skill = registry.load_skill(&name)?;
            if json {
                print_json(&skill);
            } else {
                println!("{}", serde_json::to_string_pretty(&skill)?);
                let body = registry.skill_body(&skill)?;
                println!("\n--- body ({}) ---\n{body}", skill.body_path);
            }
        }

        Command::Status => {
            let registry = open_registry(project_root)?;
            let config = registry.config()?;
            let app_version = appver::detect(project_root, &config, app_version_override)?;
            let skills = registry.list_skills()?;
            let arch = Archaeology::new(project_root).ok();
            let resolver = Resolver::new(arch.as_ref(), &app_version);
            let resolutions = resolver.resolve_all(&skills, &app_version)?;

            // Reflect decisions onto stored skill statuses.
            for res in &resolutions {
                registry.apply_resolution(res)?;
            }

            if json {
                print_json(&serde_json::json!({
                    "app_version": app_version,
                    "resolutions": resolutions,
                }));
            } else {
                println!("App version: {app_version}\n");
                if resolutions.is_empty() {
                    println!("No skills registered.");
                } else {
                    let rows = resolutions
                        .iter()
                        .map(|r| {
                            vec![
                                r.skill.clone(),
                                r.from_app.to_string(),
                                r.to_app.to_string(),
                                r.decision.to_string(),
                                r.reason.clone(),
                            ]
                        })
                        .collect();
                    print!(
                        "{}",
                        table(&["SKILL", "FROM", "TO", "DECISION", "REASON"], rows)
                    );
                }
            }
        }

        Command::Archaeology { from, to } => {
            let arch = Archaeology::new(project_root).map_err(|_| {
                SkillasticError::Other("not a git repository; archaeology unavailable".into())
            })?;
            let to_ref = match to {
                Some(t) => t,
                None => "HEAD".into(),
            };
            let from_ref = match from {
                Some(f) => f,
                None => arch.previous_tag(&to_ref).ok_or_else(|| {
                    SkillasticError::Other("no earlier tag found; pass --from".into())
                })?,
            };
            let chain = arch.analyze(&from_ref, &to_ref)?;
            if json {
                print_json(&chain);
            } else {
                println!("Commit chain {} -> {}\n", chain.from_ref, chain.to_ref);
                println!("Commits: {}", chain.commits.len());
                for c in &chain.commits {
                    println!("  {} {}", c.hash, c.subject);
                }
                if !chain.breaking.is_empty() {
                    println!("\nBreaking:");
                    for c in &chain.breaking {
                        println!("  {} {}", c.hash, c.subject);
                    }
                }
                if chain.dep_changes.any() {
                    println!("\nDependencies:");
                    for (n, v) in &chain.dep_changes.added {
                        println!("  + {n} ({v})");
                    }
                    for (n, v) in &chain.dep_changes.removed {
                        println!("  - {n} ({v})");
                    }
                    for (n, (o, new)) in &chain.dep_changes.changed {
                        println!("  ~ {n}: {o} -> {new}");
                    }
                }
                if !chain.toolchain_changes.appeared.is_empty()
                    || !chain.toolchain_changes.disappeared.is_empty()
                {
                    println!("\nToolchains:");
                    for t in &chain.toolchain_changes.appeared {
                        println!("  + {t}");
                    }
                    for t in &chain.toolchain_changes.disappeared {
                        println!("  - {t}");
                    }
                }
            }
        }

        Command::Capture => {
            let fp = Capture::scan(project_root)?;
            if json {
                print_json(&fp);
            } else {
                println!("{}", serde_json::to_string_pretty(&fp)?);
            }
        }

        Command::Migrate { name, all, dry_run } => {
            let registry = open_registry(project_root)?;
            let config = registry.config()?;
            let app_version = appver::detect(project_root, &config, app_version_override)?;
            let migrator = Migrator::new(&registry);

            let targets: Vec<String> =
                if all {
                    let arch = Archaeology::new(project_root).ok();
                    let resolver = Resolver::new(arch.as_ref(), &app_version);
                    resolver
                        .resolve_all(&registry.list_skills()?, &app_version)?
                        .into_iter()
                        .filter(|r| r.decision == Decision::Migrate)
                        .map(|r| r.skill)
                        .collect()
                } else {
                    vec![name.ok_or_else(|| {
                        SkillasticError::Other("pass a skill name or --all".into())
                    })?]
                };

            if targets.is_empty() {
                println!("Nothing to migrate.");
                return Ok(());
            }

            let mut outcomes = Vec::new();
            for target in &targets {
                let outcome = migrator.migrate(target, &app_version, dry_run)?;
                if json {
                    outcomes.push(outcome);
                } else {
                    println!(
                        "{}{}: skill {} -> {} (app {} -> {})",
                        if dry_run { "[dry-run] " } else { "" },
                        outcome.skill,
                        outcome.from_skill_version,
                        outcome.to_skill_version,
                        outcome.from_app,
                        outcome.to_app
                    );
                    println!("  reason: {}", outcome.delta.reason());
                    if let Some(dir) = &outcome.snapshot_dir {
                        println!("  snapshot: {}", dir.display());
                    }
                    if dry_run {
                        print!("{}", outcome.delta.to_markdown());
                    }
                }
            }
            if json {
                print_json(&outcomes);
            }
        }

        Command::Verify { name } => {
            let registry = open_registry(project_root)?;
            let config = registry.config()?;
            let app_version = appver::detect(project_root, &config, app_version_override)?;
            let skill = verify(&registry, &name, &app_version)?;
            if json {
                print_json(&skill);
            } else {
                println!(
                    "Verified '{}' v{} against app {app_version} (confidence: {:.2})",
                    skill.name, skill.skill_version, skill.confidence
                );
            }
        }

        Command::History { name } => {
            let registry = open_registry(project_root)?;
            let skill = registry.load_skill(&name)?;
            if json {
                let mut chain = vec![skill.clone()];
                let mut cursor = skill.parent.clone();
                while let Some(id) = cursor {
                    match registry.load_snapshot(&id) {
                        Ok((parent, _)) => {
                            cursor = parent.parent.clone();
                            chain.push(parent);
                        }
                        Err(_) => break,
                    }
                }
                print_json(&chain);
            } else {
                println!("Lineage of '{}':", name);
                let mut current_id = skill.id();
                let mut cursor = Some(skill.clone());
                while let Some(node) = cursor {
                    let marker = if node.id() == current_id {
                        " (current)"
                    } else {
                        ""
                    };
                    println!("  {}{}", node.id(), marker);
                    if !node.mutation_history.is_empty() {
                        for m in &node.mutation_history {
                            println!(
                                "    [{}] {} — {}",
                                m.timestamp.format("%Y-%m-%d"),
                                m.commit,
                                m.reason
                            );
                        }
                    }
                    cursor = match &node.parent {
                        Some(id) if *id != current_id => {
                            current_id = id.clone();
                            registry.load_snapshot(id).ok().map(|(s, _)| s)
                        }
                        _ => None,
                    };
                }
            }
        }

        Command::Daemon { interval } => {
            let registry = open_registry(project_root)?;
            Daemon::new(&registry, interval).run()?;
        }

        Command::Events => {
            let registry = open_registry(project_root)?;
            let events = recent_events(&registry)?;
            if json {
                print_json(&events);
            } else if events.is_empty() {
                println!("No daemon events recorded yet.");
            } else {
                for event in &events {
                    println!("{event}");
                }
            }
        }

        Command::Audit {
            root,
            sources,
            infer,
            model,
        } => {
            let root = if root.is_absolute() {
                root
            } else {
                project_root.join(root)
            };
            let sources = sources
                .into_iter()
                .map(|source| {
                    if source.is_absolute() {
                        source
                    } else {
                        project_root.join(source)
                    }
                })
                .collect::<Vec<_>>();
            let report = audit::run(&root, &sources, infer, &model)?;
            if json {
                print_json(&report);
            } else {
                println!(
                    "Audited {} project(s) under {}",
                    report.totals.projects, report.root
                );
                for project in &report.projects {
                    println!(
                        "{:14} {}",
                        format!("{:?}", project.workspace.state).to_lowercase(),
                        project.path
                    );
                    for reason in &project.workspace.reasons {
                        println!("  - {reason}");
                    }
                }
                if let Some(inference) = report.inference {
                    println!("Groq inference: {}", inference.status);
                }
            }
        }
    }
    Ok(())
}

fn open_registry(project_root: &Path) -> Result<Registry> {
    Registry::open(project_root)
}

fn print_json<T: Serialize>(value: &T) {
    match serde_json::to_string_pretty(value) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("error serializing output: {e}"),
    }
}

/// Minimal padded-column table renderer.
fn table(headers: &[&str], rows: Vec<Vec<String>>) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in &rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(cell.len());
            }
        }
    }
    let render = |cells: &[String]| -> String {
        cells
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{:<width$}", c, width = widths[i]))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };
    let header_cells: Vec<String> = headers.iter().map(|h| h.to_string()).collect();
    let mut out = render(&header_cells);
    out.push('\n');
    out.push_str(
        &widths
            .iter()
            .map(|w| "-".repeat(*w))
            .collect::<Vec<_>>()
            .join("  "),
    );
    out.push('\n');
    for row in rows {
        out.push_str(&render(&row));
        out.push('\n');
    }
    out
}
