# Skill: running builds (flags-cli)

How to drive the project build CLI.

## Conventions

- Builds run through `buildtool.py`:

  ```
  python3 buildtool.py --out DIR [--fast]
  ```

- `--out DIR` (required): the output directory. The build writes
  `DIR/BUILD.txt` recording which mode ran.
- `--fast`: boolean flag — pass it for a fast build. Omit it for a full
  build; there is no other way to pick a mode.
- Automation scripts always pin the output directory explicitly and run
  from the repo root, e.g. `python3 buildtool.py --out dist --fast`.

## Patterns

- Shell wrappers use `set -euo pipefail` and `cd "$(dirname "$0")"` so
  they work from any cwd.
- Check exit status: the CLI exits 0 on success.
