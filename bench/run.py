#!/usr/bin/env python3
"""bench/run.py — multi-agent benchmark runner (agents x tasks x arms).

Runs coding agents against task fixtures, captures tool-use and efficiency
metrics, grades the result, and appends one JSON line per run to
<out>/runs.jsonl. Stdlib only.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import signal
import sqlite3
import subprocess
import sys
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
from pathlib import Path

BENCH_DIR = Path(__file__).resolve().parent
FIXTURES_DIR = BENCH_DIR / "fixtures"
RESULTS_ROOT = BENCH_DIR / "results"
KIMI_HOME = Path("/tmp/kbench-home")
AGY_TRACE_DIR = Path("/home/sal/.gemini/antigravity-cli/conversations")
GRADER_TIMEOUT = 120

ARMS = ("stale", "migrated")
ARM_SKILL_FILES = {"stale": "skill_v1.md", "migrated": "skill_migrated.md"}

AGENTS = {
    "kimi-k3": {"harness": "kimi", "model": "kimi-code/k3"},
    "kimi-k2.7": {"harness": "kimi", "model": "kimi-code/kimi-for-coding"},
    # Groq models via the raw tool-loop harness (kimi/codex are provider-incompatible).
    "groq-gpt-oss-120b": {"harness": "groqraw", "model": "openai/gpt-oss-120b"},
    "groq-llama-70b": {"harness": "groqraw", "model": "llama-3.3-70b-versatile"},
    # Single-shot compound systems (built-in server-side tools, no client tools).
    "groq-compound": {"harness": "compound", "model": "groq/compound"},
    "groq-compound-mini": {"harness": "compound", "model": "groq/compound-mini"},
    "codex": {"harness": "codex", "model": None},
    "agy": {"harness": "agy", "model": None},
}

# Hidden debug harness, enabled only via --include-debug.
DEBUG_AGENTS = {
    "debug-echo": {"harness": "echo", "model": None},
}


def build_prompt(workspace: Path, task_text: str) -> str:
    return (
        f"You are an autonomous coding agent working in the project at {workspace} "
        "(absolute path — all file operations must stay inside it).\n\n"
        "The file SKILL.md documents this project's conventions; follow it.\n\n"
        f"TASK:\n{task_text}\n\n"
        "Rules: you may run the tests (python3 test_*.py) to check your work, but you "
        "must NOT modify any test file. Do not stop until the task is fully implemented. "
        "When finished, reply with a one-paragraph summary and stop."
    )


def build_compound_prompt(task_text: str) -> str:
    """Prompt body for single-shot harnesses (no filesystem access; the
    harness inlines the workspace files after this text)."""
    return (
        "You are completing a coding task. All project files are included below.\n\n"
        "The file SKILL.md documents this project's conventions; follow it.\n\n"
        f"TASK:\n{task_text}\n"
    )


# Files each task expects the agent to produce (used by single-shot harnesses).
TASK_TARGETS = {
    "canon-api": ["service.py"],
    "dep-swap": ["report.py"],
    "flags-cli": ["run_build.sh"],
    "state-format": ["config.py"],
}


def snapshot(workspace: Path) -> dict[str, str]:
    """sha256 of every file under workspace, keyed by relative path."""
    snap = {}
    for p in sorted(workspace.rglob("*")):
        if p.is_file():
            snap[str(p.relative_to(workspace))] = hashlib.sha256(p.read_bytes()).hexdigest()
    return snap


def prepare_workspace(fixture: Path, ws: Path, arm: str) -> None:
    if ws.exists():
        shutil.rmtree(ws)
    ws.mkdir(parents=True)
    app = fixture / "app"
    if app.is_dir():
        for item in app.iterdir():
            dest = ws / item.name
            if item.is_dir():
                shutil.copytree(item, dest)
            else:
                shutil.copy2(item, dest)
    shutil.copy2(fixture / ARM_SKILL_FILES[arm], ws / "SKILL.md")


def agent_argv(harness: str, model: str | None, prompt: str, workspace: Path,
               log_out: Path, targets: list[str] | None = None) -> list[str]:
    if harness == "kimi":
        return ["kimi", "-p", prompt, "--output-format", "stream-json", "-m", model]
    if harness == "codex":
        return ["codex", "exec", "--json", "-C", str(workspace), "-s", "workspace-write",
                "--skip-git-repo-check", "--ephemeral", prompt]
    if harness == "agy":
        return ["agy", "-p", prompt, "--dangerously-skip-permissions",
                "--print-timeout", "10m", "--log-file", str(log_out)]
    if harness == "groqraw":
        prompt_file = log_out.with_suffix(".prompt.txt")
        prompt_file.write_text(prompt)
        harness_py = Path(__file__).resolve().parent / "groq_harness.py"
        return [sys.executable, str(harness_py), model, str(workspace),
                str(prompt_file), str(log_out)]
    if harness == "compound":
        prompt_file = log_out.with_suffix(".prompt.txt")
        prompt_file.write_text(prompt)
        harness_py = Path(__file__).resolve().parent / "compound_harness.py"
        return [sys.executable, str(harness_py), model, str(workspace),
                str(prompt_file), str(log_out)] + (targets or [])
    if harness == "echo":
        return ["true"]
    raise ValueError(f"unknown harness: {harness}")


def run_process(argv: list[str], cwd: Path, env: dict | None, log_out: Path,
                log_err: Path, timeout: int) -> tuple[int | None, bool]:
    """Run argv, streaming stdout/stderr to log files. Kill the process group
    on timeout. Returns (exit_code, timed_out)."""
    with open(log_out, "ab") as out_f, open(log_err, "ab") as err_f:
        proc = subprocess.Popen(argv, cwd=str(cwd), env=env, stdout=out_f,
                                stderr=err_f, start_new_session=True)
        try:
            proc.wait(timeout=timeout)
            return proc.returncode, False
        except subprocess.TimeoutExpired:
            try:
                os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
            except (ProcessLookupError, PermissionError):
                pass
            proc.wait()
            return proc.returncode, True


def run_grader(fixture: Path, workspace: Path) -> tuple[int, int, list]:
    grader = fixture / "grader.py"
    try:
        proc = subprocess.run([sys.executable, str(grader), str(workspace)],
                              cwd=str(fixture), capture_output=True, text=True,
                              timeout=GRADER_TIMEOUT)
        lines = [ln for ln in proc.stdout.splitlines() if ln.strip()]
        data = json.loads(lines[-1]) if lines else None
        if not isinstance(data, dict):
            raise ValueError("no JSON line")
        return int(data.get("passed", 0)), int(data.get("total", 0)), list(data.get("details", []))
    except Exception:
        return 0, 0, ["grader error"]


def eval_checks(workspace: Path, patterns: list) -> dict[str, bool]:
    results = {}
    for pat in patterns:
        name = pat.get("name", "unnamed")
        target = workspace / pat.get("file", "")
        try:
            content = target.read_text(errors="replace")
        except OSError:
            results[name] = False
            continue
        found = re.search(pat.get("regex", ""), content) is not None
        results[name] = found if pat.get("expect") else not found
    return results


def parse_kimi_log(path: Path) -> tuple[int, dict, int]:
    """Parse kimi stream-json log -> (tool_calls_total, by_tool, tool_errors)."""
    total, by_tool, errors = 0, {}, 0
    for line in path.read_text(errors="replace").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            obj = json.loads(line)
        except json.JSONDecodeError:
            continue
        if not isinstance(obj, dict):
            continue
        tcs = obj.get("tool_calls")
        if isinstance(tcs, list):
            for tc in tcs:
                total += 1
                name = None
                if isinstance(tc, dict):
                    name = (tc.get("function") or {}).get("name")
                by_tool[name or "unknown"] = by_tool.get(name or "unknown", 0) + 1
        if obj.get("role") == "tool":
            content = obj.get("content")
            if not isinstance(content, str):
                content = json.dumps(content)
            c = content.strip()
            if (c.lower().startswith("error") or '"is_error":true' in c
                    or '"is_error": true' in c or "Tool failed" in c):
                errors += 1
    return total, by_tool, errors


def parse_codex_log(path: Path) -> tuple[int, dict, int, int | None, int | None]:
    """Parse codex JSONL log -> (total, by_type, errors, tokens_in, tokens_out)."""
    total, by_type, errors = 0, {}, 0
    tokens_in = tokens_out = None
    for line in path.read_text(errors="replace").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            obj = json.loads(line)
        except json.JSONDecodeError:
            continue
        if not isinstance(obj, dict):
            continue
        t = obj.get("type")
        if t == "item.started":
            item = obj.get("item") or {}
            it_type = item.get("type") or "unknown"
            total += 1
            by_type[it_type] = by_type.get(it_type, 0) + 1
        elif t == "item.completed":
            item = obj.get("item") or {}
            if item.get("type") == "command_execution":
                ec = item.get("exit_code")
                if ec is not None and ec != 0:
                    errors += 1
        elif t == "turn.completed":
            usage = obj.get("usage") or {}
            tokens_in = usage.get("input_tokens")
            tokens_out = usage.get("output_tokens")
    return total, by_type, errors, tokens_in, tokens_out


def parse_groqraw_log(path: Path) -> tuple[int, dict, int, int | None, int | None]:
    """Parse groq_harness JSONL -> (total, by_tool, errors, tokens_in, tokens_out)."""
    total, by_tool, errors = 0, {}, 0
    tokens_in = tokens_out = 0
    for line in path.read_text(errors="replace").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            obj = json.loads(line)
        except json.JSONDecodeError:
            continue
        t = obj.get("type")
        if t == "tool_call":
            total += 1
            name = obj.get("name") or "unknown"
            by_tool[name] = by_tool.get(name, 0) + 1
        elif t == "tool_error":
            errors += 1
        elif t == "error":
            errors += 1
        elif t == "usage":
            tokens_in += obj.get("prompt_tokens") or 0
            tokens_out += obj.get("completion_tokens") or 0
    return total, by_tool, errors, tokens_in, tokens_out


def agy_trace(since: float) -> tuple[int | None, dict, str | None]:
    """Sum step counts from agy conversation DBs modified at/after `since`.
    Returns (tool_calls_total, by_step_type, note)."""
    dbs = []
    if AGY_TRACE_DIR.is_dir():
        for db in AGY_TRACE_DIR.glob("*.db"):
            try:
                if db.stat().st_mtime >= since:
                    dbs.append(db)
            except OSError:
                pass
    if not dbs:
        return None, {}, "no agy trace"
    counts: dict[str, int] = {}
    for db in dbs:
        try:
            conn = sqlite3.connect(f"file:{db}?mode=ro&immutable=1", uri=True)
            try:
                rows = conn.execute(
                    "SELECT step_type, COUNT(*) FROM steps GROUP BY step_type").fetchall()
            finally:
                conn.close()
            for stype, n in rows:
                key = str(stype)
                counts[key] = counts.get(key, 0) + n
        except sqlite3.Error:
            pass
    return sum(counts.values()), counts, None


def run_one(agent_name: str, agent: dict, fixture: Path, task_name: str,
            arm: str, out_dir: Path, timeout: int) -> dict:
    """Execute a single (agent, task, arm) run and return its record."""
    harness = agent["harness"]
    ws = (out_dir / "ws" / f"{agent_name}__{task_name}__{arm}").resolve()
    log_out = out_dir / "logs" / f"{agent_name}__{task_name}__{arm}.out"
    log_err = out_dir / "logs" / f"{agent_name}__{task_name}__{arm}.err"
    rec = {
        "agent": agent_name,
        "harness": harness,
        "model": agent["model"],
        "task": task_name,
        "arm": arm,
        "workspace": str(ws),
        "started_at": datetime.now(timezone.utc).isoformat(),
        "duration_s": None,
        "exit_code": None,
        "timed_out": False,
        "tests_passed": 0,
        "tests_total": 0,
        "grader_details": [],
        "forbidden_edits": [],
        "checks": {},
        "tool_calls_total": None,
        "tool_calls": {},
        "tool_errors": None,
        "tokens_in": None,
        "tokens_out": None,
        "error": None,
    }
    start = time.time()
    try:
        log_out.parent.mkdir(parents=True, exist_ok=True)
        prepare_workspace(fixture, ws, arm)
        before = snapshot(ws)
        task_text = (fixture / "task.txt").read_text()
        checks_data = json.loads((fixture / "checks.json").read_text())
        prompt = build_prompt(ws, task_text)
        if harness == "compound":
            prompt = build_compound_prompt(task_text)

        env = None
        if harness == "kimi":
            home = Path(agent.get("kimi_home") or KIMI_HOME)
            if not home.is_dir():
                raise RuntimeError(f"KIMI_CODE_HOME {home} does not exist")
            env = dict(os.environ, KIMI_CODE_HOME=str(home))
        argv = agent_argv(harness, agent["model"], prompt, ws, log_out,
                          TASK_TARGETS.get(task_name, []))
        exit_code, timed_out = run_process(argv, ws, env, log_out, log_err, timeout)
        rec["exit_code"] = exit_code
        rec["timed_out"] = timed_out
        if timed_out:
            rec["error"] = f"timed out after {timeout}s"

        after = snapshot(ws)
        forbidden = set(checks_data.get("forbidden_files", []))
        rec["forbidden_edits"] = sorted(
            rel for rel, h in after.items()
            if (rel not in before or before[rel] != h) and Path(rel).name in forbidden)

        passed, total, details = run_grader(fixture, ws)
        rec["tests_passed"], rec["tests_total"], rec["grader_details"] = passed, total, details
        rec["checks"] = eval_checks(ws, checks_data.get("patterns", []))

        if harness == "kimi":
            t, calls, errs = parse_kimi_log(log_out)
            rec["tool_calls_total"], rec["tool_calls"], rec["tool_errors"] = t, calls, errs
        elif harness == "codex":
            t, calls, errs, tin, tout = parse_codex_log(log_out)
            rec["tool_calls_total"], rec["tool_calls"], rec["tool_errors"] = t, calls, errs
            rec["tokens_in"], rec["tokens_out"] = tin, tout
        elif harness == "agy":
            t, calls, note = agy_trace(start)
            rec["tool_calls_total"], rec["tool_calls"] = t, calls
            rec["tool_errors"] = 0 if t is not None else None
            if note and rec["error"] is None:
                rec["error"] = note
        elif harness == "groqraw":
            t, calls, errs, tin, tout = parse_groqraw_log(log_out)
            rec["tool_calls_total"], rec["tool_calls"], rec["tool_errors"] = t, calls, errs
            rec["tokens_in"], rec["tokens_out"] = tin, tout
        elif harness == "compound":  # same JSONL event schema as groqraw
            t, calls, errs, tin, tout = parse_groqraw_log(log_out)
            rec["tool_calls_total"], rec["tool_calls"], rec["tool_errors"] = t, calls, errs
            rec["tokens_in"], rec["tokens_out"] = tin, tout
        # echo harness: no log parsing, tool fields stay null/empty.
    except Exception as e:  # never let one run kill the batch
        rec["error"] = f"{type(e).__name__}: {e}"
    rec["duration_s"] = round(time.time() - start, 2)
    return rec


def resolve_task(name_or_path: str) -> Path:
    p = Path(name_or_path)
    if p.is_dir():
        return p.resolve()
    p = FIXTURES_DIR / name_or_path
    if p.is_dir():
        return p.resolve()
    raise SystemExit(f"error: task fixture not found: {name_or_path} "
                     f"(looked at {p} and as a direct path)")


def discover_tasks() -> list[Path]:
    if not FIXTURES_DIR.is_dir():
        return []
    return sorted(d for d in FIXTURES_DIR.iterdir() if d.is_dir())


def main() -> int:
    ap = argparse.ArgumentParser(description="Multi-agent benchmark runner.")
    ap.add_argument("--agents", help="comma-separated agent names (default: all)")
    ap.add_argument("--tasks", help="comma-separated task names or fixture paths "
                                    "(default: all under bench/fixtures)")
    ap.add_argument("--arms", help="comma-separated arms (default: stale,migrated)")
    ap.add_argument("--workers", type=int, default=3)
    ap.add_argument("--timeout", type=int, default=420, help="per-run timeout in seconds")
    ap.add_argument("--out", help="output dir (default: bench/results/<UTC timestamp>)")
    ap.add_argument("--list", action="store_true", help="print the run matrix and exit")
    ap.add_argument("--include-debug", action="store_true",
                    help="enable hidden debug agents (debug-echo)")
    args = ap.parse_args()

    registry = dict(AGENTS)
    if args.include_debug:
        registry.update(DEBUG_AGENTS)

    if args.agents:
        agent_names = [a.strip() for a in args.agents.split(",") if a.strip()]
        for a in agent_names:
            if a not in registry:
                if a in DEBUG_AGENTS:
                    raise SystemExit(f"error: agent {a!r} requires --include-debug")
                raise SystemExit(f"error: unknown agent {a!r}; known: {', '.join(registry)}")
    else:
        agent_names = list(AGENTS)

    task_dirs = ([resolve_task(t.strip()) for t in args.tasks.split(",") if t.strip()]
                 if args.tasks else discover_tasks())

    arms = ([a.strip() for a in args.arms.split(",") if a.strip()]
            if args.arms else list(ARMS))
    for a in arms:
        if a not in ARMS:
            raise SystemExit(f"error: unknown arm {a!r}; known: {', '.join(ARMS)}")

    if args.list:
        print(f"agents ({len(agent_names)}):")
        for a in agent_names:
            info = registry[a]
            print(f"  {a}  harness={info['harness']} model={info['model'] or '-'}")
        print(f"tasks ({len(task_dirs)}):")
        for t in task_dirs:
            print(f"  {t.name}  ({t})")
        print(f"arms ({len(arms)}): {', '.join(arms)}")
        print(f"matrix: {len(agent_names)} agents x {len(task_dirs)} tasks x "
              f"{len(arms)} arms = {len(agent_names) * len(task_dirs) * len(arms)} runs")
        for a in agent_names:
            for t in task_dirs:
                for arm in arms:
                    print(f"  {a} x {t.name} x {arm}")
        return 0

    if not task_dirs:
        raise SystemExit(f"error: no task fixtures found under {FIXTURES_DIR}")
    if any(registry[a]["harness"] == "kimi" for a in agent_names) and not KIMI_HOME.is_dir():
        raise SystemExit(f"error: KIMI_CODE_HOME={KIMI_HOME} does not exist; "
                         "create it before running kimi agents")

    out_dir = (Path(args.out) if args.out
               else RESULTS_ROOT / datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ"))
    out_dir = out_dir.resolve()
    (out_dir / "logs").mkdir(parents=True, exist_ok=True)
    (out_dir / "ws").mkdir(parents=True, exist_ok=True)
    runs_path = out_dir / "runs.jsonl"

    jobs = [(a, registry[a], t, t.name, arm)
            for a in agent_names for t in task_dirs for arm in arms]
    total = len(jobs)
    print(f"bench: {total} runs, workers={args.workers}, timeout={args.timeout}s, out={out_dir}",
          flush=True)

    done = 0
    with ThreadPoolExecutor(max_workers=args.workers) as ex, open(runs_path, "a") as runs_f:
        futures = {ex.submit(run_one, a, info, t, tname, arm, out_dir, args.timeout):
                   (a, tname, arm) for a, info, t, tname, arm in jobs}
        for fut in as_completed(futures):
            a, tname, arm = futures[fut]
            try:
                rec = fut.result()
            except Exception as e:  # belt-and-braces; run_one already guards
                rec = {"agent": a, "harness": registry[a]["harness"],
                       "model": registry[a]["model"], "task": tname, "arm": arm,
                       "workspace": None, "started_at": datetime.now(timezone.utc).isoformat(),
                       "duration_s": None, "exit_code": None, "timed_out": False,
                       "tests_passed": 0, "tests_total": 0, "grader_details": [],
                       "forbidden_edits": [], "checks": {}, "tool_calls_total": None,
                       "tool_calls": {}, "tool_errors": None, "tokens_in": None,
                       "tokens_out": None, "error": f"{type(e).__name__}: {e}"}
            runs_f.write(json.dumps(rec) + "\n")
            runs_f.flush()
            done += 1
            status = (f"exit={rec['exit_code']} passed={rec['tests_passed']}/"
                      f"{rec['tests_total']} dur={rec['duration_s']}s")
            if rec["timed_out"]:
                status += " TIMED_OUT"
            if rec["error"]:
                status += f" error={rec['error']}"
            print(f"[{done}/{total}] {a} x {tname} x {arm}: {status}", flush=True)

    print(f"done: {total} runs -> {runs_path}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
