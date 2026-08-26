//! `skillastic enroll`: the first-run setup path. Discovers local repos,
//! registers the ones with a `.skillastic` workspace (optionally initializing
//! plain git repos too) for daemon monitoring, installs the binary to a
//! stable location, and wires up autostart via systemd so the daemon
//! survives logout/reboot.

use crate::error::{Result, SkillasticError};
use crate::install;
use crate::model::Config;
use crate::monitor::{self, MonitorRegistry, Scope};
use crate::registry::Registry;
use serde::Serialize;
use std::path::PathBuf;

pub struct EnrollOptions {
    pub scope: Scope,
    pub roots: Vec<PathBuf>,
    pub interval: u64,
    pub init_missing: bool,
    pub max_depth: usize,
    /// Report what would happen without writing or starting anything.
    pub dry_run: bool,
}

#[derive(Debug, Serialize)]
pub struct EnrollReport {
    pub scope: String,
    pub dry_run: bool,
    pub binary_path: Option<String>,
    pub unit_path: Option<String>,
    pub enrolled: Vec<String>,
    pub initialized: Vec<String>,
    pub skipped_no_workspace: Vec<String>,
}

pub fn run(opts: EnrollOptions) -> Result<EnrollReport> {
    match run_inner(&opts) {
        Err(SkillasticError::Io(e))
            if opts.scope.requires_root() && e.kind() == std::io::ErrorKind::PermissionDenied =>
        {
            Err(SkillasticError::Other(
                "system-wide enrollment requires root; re-run with sudo".into(),
            ))
        }
        other => other,
    }
}

fn run_inner(opts: &EnrollOptions) -> Result<EnrollReport> {
    let discovered = monitor::discover_repos(&opts.roots, opts.max_depth)?;

    let mut registry = if opts.dry_run {
        MonitorRegistry::default()
    } else {
        MonitorRegistry::load(opts.scope)?
    };
    let mut enrolled = Vec::new();
    let mut initialized = Vec::new();
    let mut skipped = Vec::new();

    for repo in discovered {
        if !repo.has_skillastic {
            if opts.init_missing {
                if !opts.dry_run {
                    let name = repo
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "app".into());
                    let config = Config {
                        app_name: name,
                        ..Default::default()
                    };
                    Registry::init(&repo.path, config)?;
                }
                initialized.push(repo.path.display().to_string());
            } else {
                skipped.push(repo.path.display().to_string());
                continue;
            }
        }
        if !opts.dry_run {
            registry.upsert(&repo.path, opts.interval);
        }
        enrolled.push(repo.path.display().to_string());
    }

    if opts.dry_run {
        return Ok(EnrollReport {
            scope: opts.scope.label().into(),
            dry_run: true,
            binary_path: None,
            unit_path: None,
            enrolled,
            initialized,
            skipped_no_workspace: skipped,
        });
    }

    registry.save(opts.scope)?;
    let bin_path = install::ensure_binary_installed(opts.scope)?;
    let unit_path = install::install_unit(opts.scope, &bin_path)?;
    monitor::resume(opts.scope)?;

    Ok(EnrollReport {
        scope: opts.scope.label().into(),
        dry_run: false,
        binary_path: Some(bin_path.display().to_string()),
        unit_path: Some(unit_path.display().to_string()),
        enrolled,
        initialized,
        skipped_no_workspace: skipped,
    })
}
