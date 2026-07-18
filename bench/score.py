#!/usr/bin/env python3
"""bench/score.py — aggregate bench runs into results.csv and report.md.

Usage: python3 score.py RESULTS_DIR
Stdlib only.
"""

from __future__ import annotations

import csv
import json
import sys
from pathlib import Path

CSV_FIELDS = [
    "agent", "harness", "model", "task", "arm", "workspace", "started_at",
    "duration_s", "exit_code", "timed_out", "tests_passed", "tests_total",
    "pass_rate", "grader_details", "forbidden_edits", "checks",
    "tool_calls_total", "tool_calls", "tool_errors", "tokens_in", "tokens_out",
    "error",
]


def load_runs(results_dir: Path) -> list[dict]:
    path = results_dir / "runs.jsonl"
    if not path.is_file():
        raise SystemExit(f"error: {path} not found")
    runs = []
    for ln in path.read_text().splitlines():
        if ln.strip():
            runs.append(json.loads(ln))
    return runs


def pass_rate(run: dict) -> float:
    """Test pass rate; timeouts and errors count as 0."""
    if run.get("timed_out") or run.get("error"):
        return 0.0
    total = run.get("tests_total") or 0
    return (run.get("tests_passed") or 0) / total if total else 0.0


def mean(xs: list[float]) -> float:
    return sum(xs) / len(xs) if xs else 0.0


def fmt(x, nd=3):
    return round(x, nd) if isinstance(x, float) else x


def md_table(headers: list[str], rows: list[list]) -> str:
    lines = ["| " + " | ".join(headers) + " |",
             "|" + "|".join("---" for _ in headers) + "|"]
    for r in rows:
        lines.append("| " + " | ".join(str(c) for c in r) + " |")
    return "\n".join(lines)


def group_stats(runs: list[dict]) -> dict:
    rates = [pass_rate(r) for r in runs]
    durations = [r["duration_s"] for r in runs if r.get("duration_s") is not None]
    tools = [r["tool_calls_total"] for r in runs if r.get("tool_calls_total") is not None]
    check_vals = [v for r in runs for v in (r.get("checks") or {}).values()]
    return {
        "runs": len(runs),
        "pass_rate": mean(rates),
        "mean_duration_s": mean(durations),
        "mean_tool_calls": mean(tools) if tools else None,
        "tool_errors": sum(r.get("tool_errors") or 0 for r in runs),
        "forbidden_edits": sum(len(r.get("forbidden_edits") or []) for r in runs),
        "check_pass_rate": (sum(1 for v in check_vals if v) / len(check_vals)
                            if check_vals else None),
    }


def write_csv(runs: list[dict], path: Path) -> None:
    with open(path, "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=CSV_FIELDS)
        w.writeheader()
        for r in runs:
            row = {k: r.get(k) for k in CSV_FIELDS}
            row["pass_rate"] = round(pass_rate(r), 4)
            for k in ("grader_details", "forbidden_edits", "checks", "tool_calls"):
                row[k] = json.dumps(row[k])
            w.writerow(row)


def build_report(runs: list[dict]) -> tuple[str, str]:
    """Returns (report_markdown, headline_table_markdown)."""
    # (a) per agent x arm
    agent_arm: dict[tuple, list] = {}
    for r in runs:
        agent_arm.setdefault((r["agent"], r["arm"]), []).append(r)
    a_rows = []
    for (agent, arm), rs in sorted(agent_arm.items()):
        s = group_stats(rs)
        a_rows.append([agent, arm, s["runs"], fmt(s["pass_rate"]),
                       fmt(round(s["mean_duration_s"], 2)),
                       fmt(s["mean_tool_calls"]) if s["mean_tool_calls"] is not None else "-",
                       s["tool_errors"], s["forbidden_edits"],
                       fmt(s["check_pass_rate"]) if s["check_pass_rate"] is not None else "-"])
    headline = md_table(
        ["agent", "arm", "runs", "pass_rate", "mean_dur_s", "mean_tool_calls",
         "tool_errors", "forbidden_edits", "check_pass_rate"], a_rows)

    # (b) per task x arm
    task_arm: dict[tuple, list] = {}
    for r in runs:
        task_arm.setdefault((r["task"], r["arm"]), []).append(r)
    b_rows = []
    for (task, arm), rs in sorted(task_arm.items()):
        s = group_stats(rs)
        b_rows.append([task, arm, s["runs"], fmt(s["pass_rate"]),
                       fmt(round(s["mean_duration_s"], 2))])
    task_table = md_table(["task", "arm", "runs", "pass_rate", "mean_dur_s"], b_rows)

    # (c) skillastic effect: per agent, delta between arms
    agents = sorted({r["agent"] for r in runs})
    c_rows = []
    for agent in agents:
        per_arm = {}
        for arm in sorted({r["arm"] for r in runs if r["agent"] == agent}):
            per_arm[arm] = group_stats([r for r in runs
                                        if r["agent"] == agent and r["arm"] == arm])
        if "stale" in per_arm and "migrated" in per_arm:
            d_pass = per_arm["migrated"]["pass_rate"] - per_arm["stale"]["pass_rate"]
            d_dur = per_arm["migrated"]["mean_duration_s"] - per_arm["stale"]["mean_duration_s"]
            c_rows.append([agent,
                           fmt(per_arm["stale"]["pass_rate"]),
                           fmt(per_arm["migrated"]["pass_rate"]), fmt(d_pass),
                           fmt(round(per_arm["stale"]["mean_duration_s"], 2)),
                           fmt(round(per_arm["migrated"]["mean_duration_s"], 2)),
                           fmt(round(d_dur, 2))])
        else:
            for arm, s in per_arm.items():
                c_rows.append([agent, f"(only {arm})", fmt(s["pass_rate"]), "-",
                               fmt(round(s["mean_duration_s"], 2)), "-", "-"])
    effect_table = md_table(
        ["agent", "pass_stale", "pass_migrated", "delta_pass",
         "dur_stale_s", "dur_migrated_s", "delta_dur_s"], c_rows)

    # (d) notes: runs with error / timed_out
    notes = []
    for r in runs:
        if r.get("error") or r.get("timed_out"):
            bits = []
            if r.get("timed_out"):
                bits.append("timed_out")
            if r.get("error"):
                bits.append(f"error={r['error']}")
            notes.append(f"- `{r['agent']} x {r['task']} x {r['arm']}`: " + "; ".join(bits))
    notes_md = "\n".join(notes) if notes else "No runs with errors or timeouts."

    report = "\n\n".join([
        "# Bench report",
        f"Runs: {len(runs)}",
        "## (a) Agent x arm\n\n" + headline,
        "## (b) Task x arm\n\n" + task_table,
        "## (c) Skillastic effect (migrated - stale)\n\n" + effect_table,
        "## (d) Notes\n\n" + notes_md,
    ]) + "\n"
    return report, headline


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: python3 score.py RESULTS_DIR")
    results_dir = Path(sys.argv[1]).resolve()
    runs = load_runs(results_dir)
    if not runs:
        raise SystemExit(f"error: no runs in {results_dir / 'runs.jsonl'}")

    csv_path = results_dir / "results.csv"
    report_path = results_dir / "report.md"
    write_csv(runs, csv_path)
    report, headline = build_report(runs)
    report_path.write_text(report)

    print(f"report: {report_path}")
    print(f"csv:    {csv_path}")
    print()
    print(headline)
    return 0


if __name__ == "__main__":
    sys.exit(main())
