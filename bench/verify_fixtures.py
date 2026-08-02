#!/usr/bin/env python3
"""Verify that every benchmark reference solution satisfies its grader."""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

BENCH_DIR = Path(__file__).resolve().parent
FIXTURES_DIR = BENCH_DIR / "fixtures"


def overlay(source: Path, destination: Path) -> None:
    for item in source.iterdir():
        target = destination / item.name
        if item.is_dir():
            shutil.copytree(item, target, dirs_exist_ok=True)
        else:
            shutil.copy2(item, target)


def verify_fixture(fixture: Path, work_root: Path) -> tuple[int, int]:
    workspace = work_root / fixture.name
    workspace.mkdir()
    overlay(fixture / "app", workspace)
    overlay(fixture / "ref", workspace)

    result = subprocess.run(
        [sys.executable, str(fixture / "grader.py"), str(workspace)],
        cwd=fixture,
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )
    lines = [line for line in result.stdout.splitlines() if line.strip()]
    if result.returncode != 0 or not lines:
        raise RuntimeError(
            f"{fixture.name}: grader failed ({result.returncode}): {result.stderr.strip()}"
        )
    report = json.loads(lines[-1])
    passed = int(report.get("passed", 0))
    total = int(report.get("total", 0))
    if total == 0 or passed != total:
        details = "; ".join(report.get("details", []))
        raise RuntimeError(f"{fixture.name}: {passed}/{total} checks passed: {details}")
    return passed, total


def main() -> int:
    fixtures = sorted(path for path in FIXTURES_DIR.iterdir() if path.is_dir())
    with tempfile.TemporaryDirectory(prefix="skillastic-fixtures-") as tmp:
        work_root = Path(tmp)
        total_passed = 0
        total_checks = 0
        for fixture in fixtures:
            passed, total = verify_fixture(fixture, work_root)
            total_passed += passed
            total_checks += total
            print(f"{fixture.name}: {passed}/{total}")
    print(f"all fixtures: {total_passed}/{total_checks}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
