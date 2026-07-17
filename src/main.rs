use clap::{Parser, Subcommand};
use std::path::PathBuf;

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
        /// Start version/ref (default: latest reachable tag before --to).
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

    /// Show a skill's lineage chain.
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
}

fn main() {
    let cli = Cli::parse();
    let _ = (cli.json, cli.app_version);
    match cli.command {
        _ => {
            eprintln!("not implemented yet — engine milestones in progress");
            std::process::exit(2);
        }
    }
}
