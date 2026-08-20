//! Static templates for seeded skills, docs pages, and setup files.

/// Router skill that maps user situations to Skillastic commands.
pub const ASK_SKILLASTIC_BODY: &str = r#"# Ask Skillastic

You do not need to remember every Skillastic command. This skill maps a situation to the right command.

## Main flow: keep skills in sync with the app

1. **Set up the workspace**: `skillastic setup`
2. **Capture the current codebase fingerprint**: `skillastic capture`
3. **Add a skill**: `skillastic add <name> --compatible ">=1.0.0, <2.0.0"`
4. **Check status**: `skillastic status`
5. **Migrate when the app changes**: `skillastic migrate <name>`
6. **Verify after review**: `skillastic verify <name>`

## On-ramps

- **Not sure whether the workspace is healthy** → `skillastic doctor`
- **Many projects to audit** → `skillastic audit --root /path`
- **Need to find a skill** → `skillastic search <query>`
- **Want docs for a skill** → `skillastic show --docs <name>`
- **Working on domain language** → `skillastic domain-model`
- **Recording a decision** → `skillastic adr add "<title>"`

## Invocation

User-invoked. Type `skillastic ask-skillastic` or just remember the commands above.
"#;

/// Red-green-refactor discipline.
pub const TDD_BODY: &str = r#"# Test-Driven Development

TDD is the red → green loop. Use this skill when the user wants to build features or fix bugs test-first, mentions "red-green-refactor", or wants integration tests.

## Rules of the loop

- **Red before green.** Write the failing test first, then only enough code to pass it. Do not anticipate future tests.
- **One vertical slice at a time.** One seam, one test, one minimal implementation per cycle.
- **Refactoring is not part of the loop.** It belongs to the review stage, not the red → green cycle.

## What a good test is

Tests verify behavior through public interfaces, not implementation details. A good test reads like a specification and survives refactors.

## Seams

A **seam** is the public boundary you test at. Test only at pre-agreed seams. Before writing any test, write down the seams and confirm them with the user.

## Anti-patterns

- Implementation-coupled tests
- Tautological assertions
- Horizontal slicing (all tests first, then all implementation)
"#;

/// Two-axis review: Standards + Spec.
pub const CODE_REVIEW_BODY: &str = r#"# Code Review

Review the changes since a fixed point along two axes: **Standards** and **Spec**.

## Process

1. Pin the fixed point (commit, branch, tag, or merge-base).
2. Identify the spec source: issue references, PRD/spec files, or user-provided path.
3. Identify standards sources: `CONTRIBUTING.md`, `CODING_STANDARDS.md`, etc.
4. Run Standards and Spec reviews as parallel sub-agents so they do not pollute each other.
5. Aggregate findings under separate headings. Do not merge or rerank.

## Standards axis

Check against documented coding standards plus a baseline smell list: Mysterious Name, Duplicated Code, Feature Envy, Data Clumps, Primitive Obsession, Repeated Switches, Shotgun Surgery, Divergent Change, Speculative Generality, Message Chains, Middle Man, Refused Bequest.

## Spec axis

Check: requirements that are missing or partial, behavior that was not asked for, and requirements that look implemented but look wrong.
"#;

/// Disciplined diagnosis loop for hard bugs.
pub const DIAGNOSING_BUGS_BODY: &str = r#"# Diagnosing Bugs

Use this skill for hard bugs, intermittent flakes, and regressions between known-good states.

## Process

1. **Build a tight feedback loop.** One command that already goes red on this bug.
2. **Minimise.** Shrink the reproduction to the smallest case.
3. **Hypothesise.** List possible causes, ordered by likelihood.
4. **Instrument.** Add logging or probes to distinguish hypotheses.
5. **Fix.** Make the smallest change that fixes the bug.
6. **Regression-test.** Add a test that fails before the fix and passes after.

Do not theorise until you have a red feedback loop.
"#;

/// Turn a discussion into a spec.
pub const TO_SPEC_BODY: &str = r#"# To Spec

Turn the current conversation into a spec and publish it to the issue tracker. Do NOT interview the user again; synthesise what you already know.

## Process

1. Explore the repo to understand current state.
2. Sketch the seams at which the feature will be tested. Existing seams are preferred.
3. Write the spec using the sections below.
4. Publish to the configured issue tracker with the `ready-for-agent` label.

## Spec template

- Problem Statement
- Solution
- User Stories (numbered, extensive)
- Implementation Decisions
- Testing Decisions
- Out of Scope
- Further Notes
"#;

/// Break a plan into tracer-bullet tickets.
pub const TO_TICKETS_BODY: &str = r#"# To Tickets

Break a plan, spec, or conversation into tracer-bullet tickets, each declaring its blocking edges.

## Rules

- Each slice cuts a narrow but complete path through every layer.
- A completed slice is demoable or verifiable on its own.
- Each slice fits in a single fresh context window.
- Wide refactors use expand–contract, not forced vertical slicing.

## Ticket template

- Title
- What to build
- Acceptance criteria
- Blocked by
- Status: ready-for-agent
"#;

/// Implement work from a spec or tickets.
pub const IMPLEMENT_BODY: &str = r#"# Implement

Implement the work described by the spec or tickets.

## Process

1. Use the `tdd` skill at pre-agreed seams.
2. Run typechecking regularly, single test files regularly, and the full test suite once at the end.
3. Use the `code-review` skill to review the diff.
4. Commit the work.
"#;

/// Triage incoming issues through a state machine.
pub const TRIAGE_BODY: &str = r#"# Triage

Move issues through a small state machine of triage roles.

## Category roles

- `bug`: something is broken
- `enhancement`: new feature or improvement

## State roles

- `needs-triage`: maintainer needs to evaluate
- `needs-info`: waiting on reporter
- `ready-for-agent`: fully specified, agent can pick up
- `ready-for-human`: needs human implementation
- `wontfix`: will not be actioned

## Process

1. Gather context: read the issue/PR, explore the codebase.
2. Check redundancy and prior rejection.
3. Verify the claim: reproduce bugs, run tests for PRs.
4. Grill if needed using the domain-modeling skill.
5. Apply the outcome and post a brief.
"#;

/// Compact a session for another agent.
pub const HANDOFF_BODY: &str = r#"# Handoff

Compact the current conversation into a handoff document so another agent can continue.

## Rules

- Save to the OS temporary directory, not the workspace.
- Include a "suggested skills" section naming which skills the next agent should reach for.
- Do not duplicate content already in specs, ADRs, issues, or commits; reference them.
- Redact secrets, API keys, and personally identifiable information.
"#;

/// Human-facing docs page template.
pub const DOCS_PAGE_TEMPLATE: &str = r#"## What it does

One or two plain-language paragraphs. Lead with the skill's one-sentence job, then state the defining constraint.

## When to reach for it

Invocation mode: you type it, or the model fires it automatically.

Trigger boundary: reach for this when …

## Common questions

**Question one?**
Answer.

**Question two?**
Answer.

## It's working if

- You see this signal.
- The artifact matches this shape.
"#;

/// `CONTEXT.md` seed template.
pub const CONTEXT_MD_TEMPLATE: &str = r#"# Project context

A shared glossary for this project. Keep it free of implementation details.

## Language

**Term**:
Precise definition. _Avoid_: synonyms that muddy the term.

## Relationships

- A **Term** relates to another term like this.

## Flagged ambiguities

- Previously overloaded word X: now resolved to mean Y.
"#;

/// ADR seed template. Use `{number}` and `{slug}` and `{title}` placeholders.
pub const ADR_TEMPLATE: &str = r#"# {number}. {title}

## Status

Accepted

## Context

What forced the decision?

## Decision

What we decided.

## Consequences

What becomes easier or harder because of this decision.
"#;

/// Setup file: issue tracker workflow.
pub const ISSUE_TRACKER_MD_TEMPLATE: &str = r#"# Issue tracker

Issues for this project are tracked here.

## Provider

{provider}

## Workflow

{workflow}
"#;

/// Setup file: triage labels.
pub const TRIAGE_LABELS_MD_TEMPLATE: &str = r#"# Triage labels

The canonical triage roles and the label strings that represent them in this project's issue tracker.

| Role | Label |
| --- | --- |
| needs-triage | needs-triage |
| needs-info | needs-info |
| ready-for-agent | ready-for-agent |
| ready-for-human | ready-for-human |
| wontfix | wontfix |
"#;

/// Setup file: domain doc layout.
pub const DOMAIN_MD_TEMPLATE: &str = r#"# Domain docs

This project uses a single-context layout.

- Glossary: `CONTEXT.md` at the project root
- Architectural decisions: `.skillastic/adr/`

Agent skills should read `CONTEXT.md` for vocabulary before writing specs or code, and respect ADRs in the area being changed.
"#;

/// Project-type init templates.
pub const TEMPLATE_RUST_BODY: &str = r#"# Rust project conventions

- Use `cargo test` for the full test suite.
- Prefer `cargo clippy --all-targets --all-features -- -D warnings` before committing.
- Keep public APIs documented with rustdoc.
- Use `thiserror` for error types and `anyhow` only at application boundaries.
"#;

pub const TEMPLATE_NODE_BODY: &str = r#"# Node.js project conventions

- Use `npm test` (or `pnpm test` / `yarn test`) for the test suite.
- Run `npm run lint` and `npm run typecheck` before committing.
- Prefer explicit dependency versions in `package.json`.
"#;

pub const TEMPLATE_PYTHON_BODY: &str = r#"# Python project conventions

- Use the project's test runner (`pytest`, `python -m unittest`, etc.).
- Run the formatter and linter (`ruff`, `black`, etc.) before committing.
- Pin runtime dependencies in `pyproject.toml` or `requirements.txt`.
"#;

pub const TEMPLATE_GO_BODY: &str = r#"# Go project conventions

- Use `go test ./...` for the full test suite.
- Run `go vet ./...` before committing.
- Keep module dependencies tidy with `go mod tidy`.
"#;

/// Map a template name to its skill body.
pub fn workflow_skill_body(name: &str) -> Option<&'static str> {
    match name {
        "ask-skillastic" => Some(ASK_SKILLASTIC_BODY),
        "tdd" => Some(TDD_BODY),
        "code-review" => Some(CODE_REVIEW_BODY),
        "diagnosing-bugs" => Some(DIAGNOSING_BUGS_BODY),
        "to-spec" => Some(TO_SPEC_BODY),
        "to-tickets" => Some(TO_TICKETS_BODY),
        "implement" => Some(IMPLEMENT_BODY),
        "triage" => Some(TRIAGE_BODY),
        "handoff" => Some(HANDOFF_BODY),
        _ => None,
    }
}

/// Project-type template names.
pub const PROJECT_TEMPLATES: &[&str] = &["rust", "node", "python", "go"];

pub fn project_template_body(name: &str) -> Option<&'static str> {
    match name {
        "rust" => Some(TEMPLATE_RUST_BODY),
        "node" => Some(TEMPLATE_NODE_BODY),
        "python" => Some(TEMPLATE_PYTHON_BODY),
        "go" => Some(TEMPLATE_GO_BODY),
        _ => None,
    }
}
