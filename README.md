# Skillastic

Skillastic is a version-aware runtime for maintaining AI-agent skills alongside the applications they describe. It records skill compatibility, detects application changes, captures project context, and produces reviewable skill migrations with lineage instead of silently rewriting instructions.

## What it does

- Stores skill metadata and Markdown bodies in a local `.skillastic/` workspace.
- Organizes skills into buckets (`core`, `engineering`, `productivity`, `misc`, `in-progress`, `deprecated`) and tracks who can invoke them (user, model, or both).
- Resolves skills against application versions using deterministic semver rules.
- Inspects git history, dependency changes, and toolchain changes between releases.
- Captures a compact fingerprint of frameworks, dependencies, tools, and project shape.
- Migrates skills with immutable snapshots and an auditable mutation history.
- Seeds a domain glossary (`CONTEXT.md`) and architectural decision records (ADRs).
- Emits human-readable tables or machine-readable JSON from every workflow.

## Install

Skillastic requires a current stable Rust toolchain (edition 2024).

```sh
cargo install --path .
skillastic --version
```

### Man pages

Manual pages for every command and subcommand live in `man/` (e.g.
`man/skillastic-add.1`, `man/skillastic-monitor-add.1`). Install them with:

```sh
sudo cp man/*.1 /usr/local/share/man/man1/
```

Then view with `man skillastic` or `man skillastic-monitor-add`. The pages
are generated from the clap definitions in `src/main.rs` — regenerate them
after changing the CLI with:

```sh
cargo run --example gen-man
```

## Main flow

Run these commands in the application whose skills you want to maintain:

```sh
skillastic init --app-name my-app --app-version 1.0.0
skillastic setup              # configure issue tracker, triage labels, domain docs
skillastic doctor             # verify the workspace is healthy
skillastic add frontend \
  --version 1.0.0 \
  --compatible ">=1.0.0, <2.0.0" \
  --verify
skillastic status
skillastic capture --json
```

## On-ramps

- **Not sure which command fits?** `skillastic show --docs ask-skillastic`
- **Many projects to audit?** `skillastic audit --root /home/sal --json`
- **Working on domain language?** `skillastic domain-model`
- **Checking installed skill files?** `skillastic validate-skills ~/.agents/skills ~/.codex/skills`
- **Recording a decision?** `skillastic adr add "Use immutable snapshots"`
- **Need a workflow skill?** `skillastic add tdd --template tdd --bucket engineering --invocation model`

## Workflow skills

Skillastic ships built-in skill templates that agents can use directly:

| Template | Purpose |
| --- | --- |
| `ask-skillastic` | Router that maps situations to Skillastic commands |
| `tdd` | Red-green-refactor discipline |
| `code-review` | Two-axis review (Standards + Spec) |
| `diagnosing-bugs` | Tight feedback-loop bug diagnosis |
| `to-spec` | Turn a discussion into a spec |
| `to-tickets` | Break a plan into tracer-bullet tickets |
| `implement` | Build work from a spec or tickets |
| `triage` | State-machine issue triage |
| `handoff` | Compact a session for another agent |

Install a workflow skill with:

```sh
skillastic add tdd --template tdd --bucket engineering --invocation model
```

## Migration workflow

When the application moves outside a skill's compatibility range, preview the deterministic migration before writing it:

```sh
skillastic --app-version 2.0.0 migrate frontend --dry-run
skillastic --app-version 2.0.0 migrate frontend
skillastic verify frontend
skillastic history frontend
```

## Skill-use review and refinement suggestions

Skillastic can watch how skills are actually used and turn that signal into upkeep work:

- Agents (or hooks, or scripts) record each use: `skillastic usage record --skill frontend --event invoked` (`invoked` | `completed` | `failed`, optional `--note`). Events append to `.skillastic/usage.jsonl`.
- `skillastic review` aggregates the log and flags skills that need attention: never invoked, in use but verified against an older app version, or failing at least half of their recorded invocations. The report is written to `.skillastic/review.json`.
- The daemon runs the same review whenever the app changes or new usage events arrive, and logs what it finds (`skillastic events`). Set `auto_suggest: true` in `.skillastic/config.json` to also refresh suggestions automatically (at most once per hour).
- `skillastic suggest` asks an inference backend for concrete, advisory refinement suggestions for every flagged skill (or one named skill), grounded in the skill body, usage stats, and review findings. Results land in `.skillastic/suggestions.json` and are never applied to skill bodies automatically.

Suggestions need an inference backend, and stay off unless configured: either an external `llm_command` in `.skillastic/config.json` (payload on stdin, JSON on stdout), or `GROQ_API_KEY` in the environment. Pick the model with `--model` or `suggest_model` in the config; the default is `llama-3.1-8b-instant`.

## Workspace layout

```text
.skillastic/
  config.json                         workspace config
  state.json                          daemon state
  promoted.json                       curated promoted skill names
  review.json                         latest skill-use review report
  suggestions.json                    latest refinement suggestions per skill
  usage.jsonl                         append-only skill-use event log
  agents/
    issue-tracker.md                  issue tracker workflow
    triage-labels.md                  triage label vocabulary
    domain.md                         domain doc layout rules
  skills/<bucket>/<name>/
    meta.json                         skill object (metadata + lineage)
    body.md                           instruction body
  skills/<name>.json                  legacy skill object
  skills/<name>.md                    legacy instruction body
  snapshots/<name>@<version>/         frozen skill.json + body.md per migration
  docs/<name>.md                      human-facing docs page
  adr/NNNN-title.md                   architectural decision records
```

Skill names are restricted to ASCII letters, digits, `.`, `_`, and `-`. Migration never mutates an old snapshot. Optional LLM-assisted rewrites and suggestions are disabled by default and require an explicit `llm_command` or `GROQ_API_KEY`.

### Validate loader-facing skills

`skillastic validate-skills` accepts a `SKILL.md` file, a skill folder, or a directory whose immediate child folders are skills. It needs no initialized Skillastic workspace. It exits nonzero on findings; add `--json` for a report with `checked` and categorized `findings`.

The validator checks that each skill has a `SKILL.md` with YAML frontmatter, a nonempty instruction body, a valid folder-matching `name`, and a nonempty string `description`. It parses YAML rather than guessing from text, and checks the types of supported optional fields (`license`, `compatibility`, `allowed-tools`, and `metadata`). This catches missing frontmatter and indentation errors that prevent skill discovery. The `.skillastic` registry's `body.md` files remain plain Markdown and use `skillastic lint` for registry checks.

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
