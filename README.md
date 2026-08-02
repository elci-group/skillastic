# Skillastic

Skillastic is a version-aware runtime for maintaining AI-agent skills alongside the applications they describe. It records skill compatibility, detects application changes, captures project context, and produces reviewable skill migrations with lineage instead of silently rewriting instructions.

## What it does

- Stores skill metadata and Markdown bodies in a local `.skillastic/` workspace.
- Resolves skills against application versions using deterministic semver rules.
- Inspects git history, dependency changes, and toolchain changes between releases.
- Captures a compact fingerprint of frameworks, dependencies, tools, and project shape.
- Migrates skills with immutable snapshots and an auditable mutation history.
- Emits human-readable tables or machine-readable JSON from every workflow.

## Install

Skillastic requires a current stable Rust toolchain (edition 2024).

```sh
cargo install --path .
skillastic --version
```

## Quick start

Run these commands in the application whose skills you want to maintain:

```sh
skillastic init --app-name my-app --app-version 1.0.0
skillastic add frontend \
  --version 1.0.0 \
  --compatible ">=1.0.0, <2.0.0" \
  --verify
skillastic status
skillastic capture --json
```

When the application moves outside a skill's compatibility range, preview the deterministic migration before writing it:

```sh
skillastic --app-version 2.0.0 migrate frontend --dry-run
skillastic --app-version 2.0.0 migrate frontend
skillastic verify frontend
skillastic history frontend
```

Use `skillastic <command> --help` for command-specific options. `--json` and `--app-version` are global flags and may be placed before or after a subcommand.

## Workspace layout

```text
.skillastic/
  config.json
  state.json
  skills/<name>.json
  skills/<name>.md
  snapshots/<name>@<version>/
```

Skill names are restricted to ASCII letters, digits, `.`, `_`, and `-`. Migration never mutates an old snapshot. Optional LLM-assisted rewrites are disabled by default and require an explicit `llm_command` in `.skillastic/config.json`.

## Development

The local quality gate matches CI:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features
cargo audit
```

The benchmark harness under `bench/` measures whether migrated skills improve coding-agent outcomes. See [bench/README.md](bench/README.md) for its matrix and usage.

## Project policies

Contributions are described in [CONTRIBUTING.md](CONTRIBUTING.md). Report vulnerabilities according to [SECURITY.md](SECURITY.md). Release notes live in [CHANGELOG.md](CHANGELOG.md).

Skillastic is available under the [MIT License](LICENSE).
