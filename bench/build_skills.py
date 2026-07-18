#!/usr/bin/env python3
"""Dogfood script: generate skill_migrated.md for every fixture.

For each fixture in bench/fixtures/NAME:

1. Create a throwaway git repo (tempfile.TemporaryDirectory), commit the
   fixture's v1/ sources and tag v1.0.0.
2. Run `skillastic init`, then `skillastic add NAME-skill --version 1.0.0
   --compatible ">=1.0.0, <2.0.0" --body fixtures/NAME/skill_v1.md --verify`.
3. Replace the tree with the v2 app/ sources, commit with a conventional
   breaking-change subject that names the drift, tag v2.0.0.
4. Run `skillastic migrate NAME-skill` and copy the resulting
   .skillastic/skills/NAME-skill.md to fixtures/NAME/skill_migrated.md.

Idempotent: fixtures with an existing skill_migrated.md are skipped
unless --force is given.

Usage: python3 bench/build_skills.py [--force]
"""

import argparse
import pathlib
import shutil
import subprocess
import sys
import tempfile

BENCH_DIR = pathlib.Path(__file__).resolve().parent
FIXTURES_DIR = BENCH_DIR / "fixtures"
SKILLASTIC = BENCH_DIR.parent / "target" / "debug" / "skillastic"

FIXTURES = {
    "canon-api": {
        "v1_subject": "v1: customer data API with canonical store and query() dicts",
        "v2_subject": "feat(api)!: replace query() dicts with Client Row "
                      "canonicalization (NFC, zero-width strip, casefold)",
    },
    "dep-swap": {
        "v1_subject": "v1: textutil.pad formatting helper",
        "v2_subject": "feat(fmt)!: replace textutil.pad with display-width "
                      "fmt.align (combining-mark aware)",
    },
    "flags-cli": {
        "v1_subject": "v1: buildtool CLI with --out and --fast",
        "v2_subject": "feat(cli)!: replace --out/--fast with --output-dir DIR "
                      "and --mode {fast,full}",
    },
    "state-format": {
        "v1_subject": "v1: INI configuration via configparser",
        "v2_subject": "feat(config)!: switch settings from INI to nested "
                      "settings.json (server/features sections)",
    },
}


def run(cmd, cwd, label):
    proc = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True)
    if proc.returncode != 0:
        raise RuntimeError(
            f"{label} failed (exit {proc.returncode}): {' '.join(map(str, cmd))}\n"
            f"stdout: {proc.stdout.strip()}\nstderr: {proc.stderr.strip()}"
        )
    return proc.stdout.strip()


def git(repo, *args):
    return run(
        ["git", "-c", "user.name=bench", "-c", "user.email=bench@local", *args],
        cwd=repo,
        label="git",
    )


def copy_tree_contents(src, dst):
    for entry in sorted(pathlib.Path(src).iterdir()):
        target = pathlib.Path(dst) / entry.name
        if entry.is_dir():
            shutil.copytree(entry, target, dirs_exist_ok=True)
        else:
            shutil.copy2(entry, target)


def clear_worktree(repo):
    for entry in pathlib.Path(repo).iterdir():
        if entry.name in (".git", ".skillastic"):
            continue
        if entry.is_dir():
            shutil.rmtree(entry)
        else:
            entry.unlink()


def build_fixture(name, spec):
    fixture = FIXTURES_DIR / name
    skill = f"{name}-skill"
    with tempfile.TemporaryDirectory(prefix=f"skillastic-bench-{name}-") as tmp:
        repo = pathlib.Path(tmp)

        # v1 commit.
        git(repo, "init", "-q", "-b", "main")
        (repo / ".gitignore").write_text(".skillastic/\n")
        copy_tree_contents(fixture / "v1", repo)
        git(repo, "add", "-A")
        git(repo, "commit", "-q", "-m", spec["v1_subject"])
        git(repo, "tag", "v1.0.0")

        # Register the v1 skill, verified against app v1.0.0.
        run([str(SKILLASTIC), "init"], cwd=repo, label="skillastic init")
        run(
            [
                str(SKILLASTIC), "add", skill,
                "--version", "1.0.0",
                "--compatible", ">=1.0.0, <2.0.0",
                "--body", str(fixture / "skill_v1.md"),
                "--verify",
            ],
            cwd=repo,
            label="skillastic add",
        )

        # v2 commit: the current app tree, as a breaking change.
        clear_worktree(repo)
        copy_tree_contents(fixture / "app", repo)
        git(repo, "add", "-A")
        git(repo, "commit", "-q", "-m", spec["v2_subject"])
        git(repo, "tag", "v2.0.0")

        # Migrate the skill to v2 and harvest the result.
        run([str(SKILLASTIC), "migrate", skill], cwd=repo, label="skillastic migrate")
        migrated = repo / ".skillastic" / "skills" / f"{skill}.md"
        if not migrated.is_file():
            raise RuntimeError(f"{name}: expected migrated skill at {migrated}")
        content = migrated.read_text()

        # Guard: the migration notes must name the breaking commit.
        problems = []
        if "## Migration Notes" not in content:
            problems.append("no '## Migration Notes' section")
        if "### Breaking changes" not in content:
            problems.append("no '### Breaking changes' section")
        if spec["v2_subject"] not in content:
            problems.append("breaking commit subject not listed")
        if problems:
            raise RuntimeError(f"{name}: migrated skill is wrong: {'; '.join(problems)}")

        out = fixture / "skill_migrated.md"
        shutil.copy2(migrated, out)
        return out, len(content)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--force", action="store_true",
                        help="regenerate even where skill_migrated.md exists")
    args = parser.parse_args()

    if not SKILLASTIC.is_file():
        sys.exit(f"skillastic binary not found at {SKILLASTIC} (run `cargo build`)")

    results = []
    for name, spec in sorted(FIXTURES.items()):
        out = FIXTURES_DIR / name / "skill_migrated.md"
        if out.exists() and not args.force:
            results.append(f"[skip] {name}: {out} exists (pass --force to regenerate)")
            continue
        path, size = build_fixture(name, spec)
        results.append(f"[ok]   {name}: wrote {path} ({size} bytes)")

    print("skill_migrated.md generation summary:")
    for line in results:
        print(" ", line)


if __name__ == "__main__":
    main()
