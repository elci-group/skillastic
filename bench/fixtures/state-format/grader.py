#!/usr/bin/env python3
"""Deterministic grader for the state-format fixture.

Usage: python3 grader.py WORKSPACE_PATH

Loads the shipped test module(s) from the workspace and runs them with
unittest. Prints exactly one final stdout line of JSON:
{"passed": int, "total": int, "details": [str, ...]}
Import errors or missing files count as failed tests; the grader itself
never crashes.
"""

import contextlib
import importlib.util
import json
import pathlib
import sys
import traceback
import unittest

# test file -> number of test methods it ships with
TEST_FILES = {"test_config.py": 3}
EXPECTED_TOTAL = sum(TEST_FILES.values())


def _load_module(path, name):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def run():
    workspace = pathlib.Path(sys.argv[1]).resolve()
    sys.path.insert(0, str(workspace))
    details = []
    passed = 0
    ran = 0
    for fname, expected in sorted(TEST_FILES.items()):
        fpath = workspace / fname
        if not fpath.is_file():
            details.append(f"{fname}: file missing; its {expected} tests count as failed")
            continue
        try:
            module = _load_module(fpath, fname[:-3])
        except Exception:
            short = "".join(traceback.format_exc(limit=2)).strip().splitlines()[-1]
            details.append(f"{fname}: import error ({short}); its {expected} tests count as failed")
            continue
        suite = unittest.defaultTestLoader.loadTestsFromModule(module)
        result = unittest.TestResult()
        suite.run(result)
        ok = result.testsRun - len(result.failures) - len(result.errors)
        passed += ok
        ran += result.testsRun
        details.append(f"{fname}: {ok}/{result.testsRun} passed")
        for test, _ in result.failures:
            details.append(f"FAIL {test.id()}")
        for test, _ in result.errors:
            details.append(f"ERROR {test.id()}")
    total = max(ran, EXPECTED_TOTAL)
    return {"passed": min(passed, total), "total": total, "details": details}


def main():
    try:
        with contextlib.redirect_stdout(sys.stderr):
            outcome = run()
    except Exception:
        outcome = {
            "passed": 0,
            "total": EXPECTED_TOTAL,
            "details": ["grader error: " + traceback.format_exc(limit=2).strip().splitlines()[-1]],
        }
    print(json.dumps(outcome))


if __name__ == "__main__":
    main()
