#!/usr/bin/env python3
"""Deterministic grader for the flags-cli fixture.

Usage: python3 grader.py WORKSPACE_PATH

Runs the agent-written run_build.sh inside the workspace and checks the
build output. Prints exactly one final stdout line of JSON:
{"passed": int, "total": int, "details": [str, ...]}
A missing/broken script counts as failed tests; the grader itself never
crashes.
"""

import contextlib
import json
import os
import pathlib
import shutil
import subprocess
import sys
import traceback

TOTAL = 3


def run():
    workspace = pathlib.Path(sys.argv[1]).resolve()
    details = []
    passed = 0
    script = workspace / "run_build.sh"

    # Deterministic starting state: no prior build output.
    shutil.rmtree(workspace / "dist", ignore_errors=True)

    # 1. Script exists and is executable.
    if script.is_file() and os.access(script, os.X_OK):
        passed += 1
        details.append("run_build.sh exists and is executable")
    else:
        details.append("run_build.sh is missing or not executable")

    # 2. Script runs and exits 0.
    try:
        proc = subprocess.run(
            [str(script)], cwd=workspace, capture_output=True, text=True, timeout=60
        )
        if proc.returncode == 0:
            passed += 1
            details.append("run_build.sh exited 0")
        else:
            details.append(
                f"run_build.sh exited {proc.returncode}: "
                + (proc.stderr.strip() or proc.stdout.strip())[:200]
            )
    except Exception as exc:
        details.append(f"run_build.sh could not be executed: {exc}")

    # 3. dist/BUILD.txt records a fast-mode build.
    build_file = workspace / "dist" / "BUILD.txt"
    try:
        content = build_file.read_text()
        if content.strip() == "mode=fast":
            passed += 1
            details.append("dist/BUILD.txt contains mode=fast")
        else:
            details.append(f"dist/BUILD.txt has unexpected content: {content!r}")
    except Exception as exc:
        details.append(f"dist/BUILD.txt unreadable: {exc}")

    return {"passed": passed, "total": TOTAL, "details": details}


def main():
    try:
        with contextlib.redirect_stdout(sys.stderr):
            outcome = run()
    except Exception:
        outcome = {
            "passed": 0,
            "total": TOTAL,
            "details": ["grader error: " + traceback.format_exc(limit=2).strip().splitlines()[-1]],
        }
    print(json.dumps(outcome))


if __name__ == "__main__":
    main()
