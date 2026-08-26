//! `skillastic register`: create (or reuse) a `.skillastic` workspace for
//! one or more projects and index them into the monitor registry so the
//! daemon supervisor picks them up.
//!
//! With `--mandatory`, instead of registering only the given paths, this
//! recursively scans the home directory (or `/home` for system scope) plus
//! any extra roots from the user's global config
//! ([`Scope::global_config_path`]), registering every project it finds.
//! Opt a directory out with a [`monitor::IGNORE_MARKER`] file.

use crate::error::{Result, SkillasticError};
use crate::model::Config;
use crate::monitor::{self, GlobalConfig, MonitorRegistry, Scope};
use crate::registry::Registry;
use serde::Serialize;
use std::path::{Path, PathBuf};

pub struct RegisterOptions {
    /// Explicit project directories (targeted mode), or extra scan roots on
    /// top of the defaults (mandatory mode).
    pub paths: Vec<PathBuf>,
    pub mandatory: bool,
    pub scope: Scope,
    pub interval: u64,
    /// Only used with `mandatory`.
    pub max_depth: usize,
    /// Report what would happen without writing or starting anything.
    pub dry_run: bool,
}

#[derive(Debug, Serialize)]
pub struct RegisterReport {
    pub mandatory: bool,
    pub dry_run: bool,
    /// Projects newly added to the registry.
    pub registered: Vec<String>,
    /// Projects that got a fresh `.skillastic` workspace.
    pub initialized: Vec<String>,
    /// Projects that were already registered (interval refreshed).
    pub already_indexed: Vec<String>,
}

pub fn run(opts: RegisterOptions) -> Result<RegisterReport> {
    if opts.mandatory {
        run_mandatory(&opts)
    } else {
        run_targeted(&opts)
    }
}

fn run_targeted(opts: &RegisterOptions) -> Result<RegisterReport> {
    let targets = if opts.paths.is_empty() {
        vec![std::env::current_dir()?]
    } else {
        opts.paths.clone()
    };
    for path in &targets {
        if !path.is_dir() {
            return Err(SkillasticError::Other(format!(
                "not a directory: {}",
                path.display()
            )));
        }
    }

    let mut registry = load_registry(opts)?;
    let mut registered = Vec::new();
    let mut initialized = Vec::new();
    let mut already_indexed = Vec::new();

    for path in &targets {
        let path = path.canonicalize().unwrap_or_else(|_| path.clone());
        let has_skillastic = path.join(".skillastic").is_dir();
        index_one(
            opts,
            &mut registry,
            &path,
            has_skillastic,
            &mut registered,
            &mut initialized,
            &mut already_indexed,
        )?;
    }

    finish(opts, registry, registered, initialized, already_indexed)
}

fn run_mandatory(opts: &RegisterOptions) -> Result<RegisterReport> {
    let global = GlobalConfig::load(opts.scope).unwrap_or_default();
    let mut roots = vec![monitor::default_scan_root(opts.scope)?];
    roots.extend(global.extra_roots);
    roots.extend(opts.paths.clone());
    roots.sort();
    roots.dedup();

    let discovered = monitor::discover_repos(&roots, opts.max_depth)?;

    let mut registry = load_registry(opts)?;
    let mut registered = Vec::new();
    let mut initialized = Vec::new();
    let mut already_indexed = Vec::new();

    for repo in discovered {
        index_one(
            opts,
            &mut registry,
            &repo.path,
            repo.has_skillastic,
            &mut registered,
            &mut initialized,
            &mut already_indexed,
        )?;
    }

    finish(opts, registry, registered, initialized, already_indexed)
}

fn load_registry(opts: &RegisterOptions) -> Result<MonitorRegistry> {
    if opts.dry_run {
        Ok(MonitorRegistry::default())
    } else {
        MonitorRegistry::load(opts.scope)
    }
}

/// Ensures `path` has a `.skillastic` workspace and is indexed in
/// `registry`, recording the outcome into the report buffers.
fn index_one(
    opts: &RegisterOptions,
    registry: &mut MonitorRegistry,
    path: &Path,
    has_skillastic: bool,
    registered: &mut Vec<String>,
    initialized: &mut Vec<String>,
    already_indexed: &mut Vec<String>,
) -> Result<()> {
    let already = registry.contains(path);
    if !has_skillastic {
        if !opts.dry_run {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "app".into());
            Registry::init(
                path,
                Config {
                    app_name: name,
                    ..Default::default()
                },
            )?;
        }
        initialized.push(path.display().to_string());
    }
    if !opts.dry_run {
        registry.upsert(path, opts.interval);
    }
    if already {
        already_indexed.push(path.display().to_string());
    } else {
        registered.push(path.display().to_string());
    }
    Ok(())
}

fn finish(
    opts: &RegisterOptions,
    registry: MonitorRegistry,
    registered: Vec<String>,
    initialized: Vec<String>,
    already_indexed: Vec<String>,
) -> Result<RegisterReport> {
    if !opts.dry_run {
        registry.save(opts.scope)?;
        monitor::resume(opts.scope)?;
    }
    Ok(RegisterReport {
        mandatory: opts.mandatory,
        dry_run: opts.dry_run,
        registered,
        initialized,
        already_indexed,
    })
}
