#!/usr/bin/env python3
"""Single-shot harness for Groq compound models (built-in server-side tools).

Compound models reject client `tools` but auto-invoke built-in tools
(python, web search) and report them in `executed_tools`. They cannot
touch the local filesystem, so the prompt inlines the workspace files and
the model must emit solution files as fenced code blocks whose first line
is `# file: <name>`.

Usage: python3 compound_harness.py MODEL WORKSPACE TASK_PROMPT_FILE LOG_JSONL ALLOWED_FILE [ALLOWED_FILE...]
Exit 0 on success (files emitted), 1 on provider/extraction error.
"""
import json
import os
import re
import subprocess
import sys
import time
import urllib.request

MAX_RETRIES = 5
REQ_TIMEOUT = 180


def api_key():
    cfg = open(os.path.expanduser("~/.kimi-code/config.toml")).read()
    m = re.search(r'\[providers\.groq\].*?api_key\s*=\s*"([^"]+)"', cfg, re.S)
    if not m:
        sys.exit("groq api_key not found in kimi config")
    return m.group(1)


def build_prompt(ws, task_prompt, allowed):
    parts = [task_prompt, "\n\nThe project files follow.\n"]
    for p in sorted(os.listdir(ws)):
        fp = os.path.join(ws, p)
        if os.path.isfile(fp) and not p.startswith('.'):
            with open(fp, errors="replace") as f:
                parts.append(f"\n### {p}\n```\n{f.read()}\n```\n")
    parts.append(
        "\nRespond with the final contents of ONLY these files: "
        + ", ".join(allowed)
        + ". Emit each file as one fenced code block whose first line is exactly"
        + " `# file: <name>`. Do not emit test files or modified fixtures."
        + " You may use your built-in python tool to verify your solution first.\n")
    return "".join(parts)


def extract_files(text, allowed, ws, log):
    written = []
    for m in re.finditer(r"```[^\n]*\n(# file: ([^\n]+?))?\n(.*?)```", text, re.S):
        fname = (m.group(2) or "").strip()
        if not fname:
            continue
        fname = os.path.basename(fname)
        if fname not in allowed:
            log.write(json.dumps({"type": "tool_error", "error": f"rejected non-allowed file {fname}"}) + "\n")
            continue
        with open(os.path.join(ws, fname), "w") as f:
            f.write(m.group(3))
        if fname.endswith(".sh"):
            os.chmod(os.path.join(ws, fname), 0o755)
        written.append(fname)
        log.write(json.dumps({"type": "file_written", "file": fname, "bytes": len(m.group(3))}) + "\n")
    return written


def main():
    model, ws, prompt_file, log_path = sys.argv[1:5]
    allowed = sys.argv[5:]
    task_prompt = open(prompt_file).read()
    prompt = build_prompt(ws, task_prompt, allowed)
    key = api_key()
    log = open(log_path, "a")

    body = json.dumps({"model": model, "temperature": 0, "messages": [
        {"role": "system", "content": "You are an expert coding agent. Follow the project's documented conventions precisely."},
        {"role": "user", "content": prompt},
    ]}).encode()

    resp = None
    for attempt in range(MAX_RETRIES):
        req = urllib.request.Request(
            "https://api.groq.com/openai/v1/chat/completions", data=body,
            headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json",
                     "User-Agent": "skillastic-bench/1.0"})
        try:
            resp = json.load(urllib.request.urlopen(req, timeout=REQ_TIMEOUT))
            break
        except urllib.error.HTTPError as e:
            err = e.read().decode()[:400]
            log.write(json.dumps({"type": "error", "status": e.code, "body": err}) + "\n")
            if e.code in (429, 500, 502, 503) and attempt < MAX_RETRIES - 1:
                time.sleep(8 * (attempt + 1))
                continue
            return 1

    usage = resp.get("usage") or {}
    log.write(json.dumps({"type": "usage",
                          "prompt_tokens": usage.get("prompt_tokens"),
                          "completion_tokens": usage.get("completion_tokens"),
                          "total_time_s": usage.get("total_time")}) + "\n")
    msg = resp["choices"][0]["message"]
    for tool in msg.get("executed_tools") or []:
        log.write(json.dumps({"type": "tool_call", "name": tool.get("type"),
                              "args": (tool.get("arguments") or "")[:300],
                              "output": (tool.get("output") or "")[:300]}) + "\n")
    reasoning = msg.get("reasoning") or ""
    log.write(json.dumps({"type": "reasoning_chars", "n": len(reasoning)}) + "\n")
    content = msg.get("content") or ""
    log.write(json.dumps({"type": "final_text", "content": content[:4000]}) + "\n")

    written = extract_files(content, allowed, ws, log)
    if not written:
        log.write(json.dumps({"type": "error", "body": "no allowed file emitted"}) + "\n")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
