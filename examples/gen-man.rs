// Copyright (c) 2026 sal
// SPDX-License-Identifier: MIT
//! Regenerates the man pages in `man/` from the real `clap` definitions in
//! `src/main.rs`, so the shipped man pages can never drift from the actual
//! CLI surface. Run after changing any command or subcommand:
//!
//!   cargo run --example gen-man
//!
//! One page is written per subcommand path: `man/skillastic.1`,
//! `man/skillastic-monitor.1`, `man/skillastic-monitor-add.1`, and so on.
use clap::{Command, CommandFactory};
use std::path::Path;

// The CLI types live in the binary crate; include it as a module so the
// derive output comes straight from the real definitions.
#[path = "../src/main.rs"]
mod defs;

fn write_man(cmd: &Command, dest: &Path) -> std::io::Result<()> {
    let man = clap_mangen::Man::new(cmd.clone());
    let mut buf = Vec::new();
    man.render(&mut buf)?;
    std::fs::write(dest, buf)
}

/// Render `cmd` to `man/<path-with-dashes>.1`, recursing into subcommands
/// with the full invocation path as the page name (e.g. `skillastic docs`).
fn gen_pages(cmd: &Command, path: &str, man_dir: &Path) -> std::io::Result<()> {
    let page = man_dir.join(format!("{}.1", path.replace(' ', "-")));
    write_man(cmd, &page)?;
    println!("Wrote {}", page.display());

    for sub in cmd.get_subcommands() {
        let full = format!("{} {}", path, sub.get_name());
        // `clap::Command::name` only accepts `&'static str`; leaking the
        // generated page title is fine for a one-shot generator.
        let name: &'static str = Box::leak(full.clone().into_boxed_str());
        let named = sub.clone().name(name);
        gen_pages(&named, &full, man_dir)?;
    }
    Ok(())
}

fn main() -> std::io::Result<()> {
    let man_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("man");
    std::fs::create_dir_all(&man_dir)?;
    gen_pages(&defs::Cli::command(), "skillastic", &man_dir)
}
