# Changelog

All notable changes are documented here. The project follows Semantic Versioning.

## Unreleased

### Added

- Skill taxonomy: `invocation` (user/model/both), `bucket`, and `requires` metadata on every skill.
- Bucket workspace layout: `skills/<bucket>/<name>/meta.json` + `body.md`, with backward-compatible loading of the legacy flat layout.
- Promoted-set manifest (`.skillastic/promoted.json`) and `skillastic promoted` validation.
- `ask-skillastic` router skill seeded on every `init`.
- `skillastic setup` wizard for issue tracker, triage labels, and domain doc configuration.
- `skillastic doctor` workspace health checks.
- `skillastic lint` with invocation/description checks and optional domain-language checks.
- `skillastic domain-model` and `skillastic adr add` for `CONTEXT.md` and ADRs.
- `skillastic docs generate` and `show --docs` for human-facing skill docs pages.
- `skillastic search` for finding skills by name, bucket, or body text.
- `skillastic add --template` with built-in workflow skills: `tdd`, `code-review`, `diagnosing-bugs`, `to-spec`, `to-tickets`, `implement`, `triage`, `handoff`.
- `skillastic init --template` for Rust, Node, Python, and Go project seeds.
- `skillastic remove` and automatic bucket README regeneration.
- Skill dependency checking in the resolver: missing dependencies resolve to `Incompatible`.

### Changed

- Public documentation reorganized around the main flow and on-ramps.
- Integration tests cover setup, doctor, templates, domain modeling, ADRs, docs, search, and promoted-set validation.

### Changed

- Repository and kaptaind ignore policies now exclude generated build and Python cache artifacts.

## 0.21.9 - 2026-08-01

- Internal patch release produced by kaptaind.
