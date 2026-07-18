#!/usr/bin/env python3
"""Minimal OpenAI-compatible tool-loop harness for Groq models.

Used by the bench to measure raw model tool-use accuracy/efficiency when
CLI harnesses (kimi, codex) are incompatible with the provider.

Usage: python3 groq_harness.py MODEL WORKSPACE PROMPT_FILE LOG_JSONL
Exit 0 on clean finish, 1 on provider/usage error. Max 30 tool steps.
"""
import json
import os
import re
import subprocess
import sys
import urllib.request

MAX_STEPS = 30
REQ_TIMEOUT = 90

TOOLS = [
    {"type": "function", "function": {
        "name": "read_file", "description": "Read a file relative to the workspace.",
        "parameters": {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}}},
    {"type": "function", "function": {
        "name": "write_file", "description": "Write a file relative to the workspace (overwrites).",
        "parameters": {"type": "object", "properties": {"path": {"type": "string"}, "content": {"type": "string"}}, "required": ["path", "content"]}}},
    {"type": "function", "function": {
        "name": "list_files", "description": "List all files in the workspace recursively.",
        "parameters": {"type": "object", "properties": {}}}},
    {"type": "function", "function": {
        "name": "run_command", "description": "Run a shell command in the workspace (bash -c).",
        "parameters": {"type": "object", "properties": {"command": {"type": "string"}}, "required": ["command"]}}},
    {"type": "function", "function": {
        "name": "finish", "description": "Declare the task complete.",
        "parameters": {"type": "object", "properties": {"summary": {"type": "string"}}, "required": ["summary"]}}},
    # gpt-oss on Groq sometimes emits a 'commentary' call not in the schema;
    # registering it as a no-op keeps the provider from rejecting the turn.
    {"type": "function", "function": {
        "name": "commentary", "description": "No-op channel for intermediate remarks.",
        "parameters": {"type": "object", "properties": {"text": {"type": "string"}}}}},
]


def api_key():
    cfg = open(os.path.expanduser("~/.kimi-code/config.toml")).read()
    m = re.search(r'\[providers\.groq\].*?api_key\s*=\s*"([^"]+)"', cfg, re.S)
    if not m:
        sys.exit("groq api_key not found in kimi config")
    return m.group(1)


def safe_path(ws, rel):
    p = os.path.realpath(os.path.join(ws, rel))
    if not (p == os.path.realpath(ws) or p.startswith(os.path.realpath(ws) + os.sep)):
        raise ValueError(f"path escapes workspace: {rel}")
    return p


def execute(ws, name, args):
    try:
        if name == "read_file":
            with open(safe_path(ws, args["path"]), errors="replace") as f:
                return f.read()[:20000]
        if name == "write_file":
            p = safe_path(ws, args["path"])
            os.makedirs(os.path.dirname(p), exist_ok=True)
            with open(p, "w") as f:
                f.write(args["content"])
            return f"wrote {len(args['content'])} bytes to {args['path']}"
        if name == "list_files":
            out = []
            for root, dirs, files in os.walk(ws):
                dirs[:] = [d for d in dirs if not d.startswith('.')]
                for fn in files:
                    out.append(os.path.relpath(os.path.join(root, fn), ws))
            return "\n".join(sorted(out)) or "(empty)"
        if name == "run_command":
            r = subprocess.run(["bash", "-c", args["command"]], cwd=ws,
                               capture_output=True, text=True, timeout=60)
            return (r.stdout + r.stderr)[:8000] + f"\n[exit={r.returncode}]"
        if name == "finish":
            return "__FINISH__"
        return f"error: unknown tool {name}"
    except Exception as e:  # noqa: BLE001 - surfaced to the model as tool output
        return f"error: {e}"


def main():
    model, ws, prompt_file, log_path = sys.argv[1:5]
    prompt = open(prompt_file).read()
    key = api_key()
    log = open(log_path, "a")
    messages = [
        {"role": "system", "content": "You are an autonomous coding agent. Use the provided tools to complete the task. Call `finish` when done."},
        {"role": "user", "content": prompt},
    ]
    for step in range(MAX_STEPS):
        body = json.dumps({"model": model, "messages": messages, "tools": TOOLS,
                           "tool_choice": "auto", "parallel_tool_calls": False,
                           "temperature": 0}).encode()
        req = urllib.request.Request(
            "https://api.groq.com/openai/v1/chat/completions", data=body,
            headers={"Authorization": f"Bearer {key}", "Content-Type": "application/json",
                     "User-Agent": "skillastic-bench/1.0"})
        try:
            resp = json.load(urllib.request.urlopen(req, timeout=REQ_TIMEOUT))
        except urllib.error.HTTPError as e:
            log.write(json.dumps({"type": "error", "status": e.code, "body": e.read().decode()[:500]}) + "\n")
            return 1
        usage = resp.get("usage") or {}
        log.write(json.dumps({"type": "usage", "prompt_tokens": usage.get("prompt_tokens"),
                              "completion_tokens": usage.get("completion_tokens")}) + "\n")
        msg = resp["choices"][0]["message"]
        messages.append(msg)
        calls = msg.get("tool_calls") or []
        if not calls:
            log.write(json.dumps({"type": "final_text", "content": (msg.get("content") or "")[:2000]}) + "\n")
            return 0
        for call in calls:
            name = call["function"]["name"]
            try:
                args = json.loads(call["function"].get("arguments") or "{}")
            except json.JSONDecodeError:
                args = {}
                log.write(json.dumps({"type": "tool_error", "error": "malformed arguments", "raw": call["function"].get("arguments", "")[:300]}) + "\n")
            log.write(json.dumps({"type": "tool_call", "name": name, "args": args}) + "\n")
            result = execute(ws, name, args)
            log.write(json.dumps({"type": "tool_result", "name": name, "result": result[:2000]}) + "\n")
            if result == "__FINISH__":
                return 0
            messages.append({"role": "tool", "tool_call_id": call["id"], "content": result})
    log.write(json.dumps({"type": "error", "body": "max steps exceeded"}) + "\n")
    return 1


if __name__ == "__main__":
    sys.exit(main())
