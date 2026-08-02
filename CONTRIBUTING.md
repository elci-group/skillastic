# Contributing

Thank you for improving Skillastic. Keep changes focused, deterministic, and reviewable.

## Development workflow

1. Create a branch from `main` and describe one coherent change.
2. Add or update tests for behavior changes.
3. Run the complete local quality gate documented in `README.md`.
4. Update `CHANGELOG.md` when a user-visible behavior changes.
5. Open a pull request that explains the motivation, behavior, and verification evidence.

Do not commit generated build output, benchmark results, Python bytecode, credentials, or `.kaptaind/` runtime state. Public APIs should include rustdoc, and errors should remain actionable without exposing sensitive file contents.

By contributing, you agree that your contribution is licensed under the MIT License.
