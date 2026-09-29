#!/usr/bin/env python3
"""Replays examples/calls/*.json against bg-spec-mcp over stdio and writes the structured
responses to examples/responses/. Usage: python3 examples/capture.py <bg-spec-mcp> <config>

Arrays longer than MAX_ITEMS are cut and end with {"_truncated_items": <count>}."""
import json, pathlib, subprocess, sys

MAX_ITEMS = 10

binary, config = sys.argv[1], sys.argv[2]
root = pathlib.Path(__file__).parent
proc = subprocess.Popen([binary, "--config", config], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                        stderr=subprocess.DEVNULL, text=True)
seq = 0

def send(method, params, notify=False):
    global seq
    msg = {"jsonrpc": "2.0", "method": method, "params": params}
    if not notify:
        seq += 1
        msg["id"] = seq
    proc.stdin.write(json.dumps(msg) + "\n")
    proc.stdin.flush()
    return None if notify else json.loads(proc.stdout.readline())

send("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                    "clientInfo": {"name": "capture", "version": "1"}})
send("notifications/initialized", {}, notify=True)

def scrub(v):
    if isinstance(v, dict):
        return {k: ("<timestamp>" if k in ("indexed_at_unix", "last_indexed_at_unix") else scrub(x)) for k, x in v.items()}
    if isinstance(v, list):
        items = [scrub(x) for x in v[:MAX_ITEMS]]
        if len(v) > MAX_ITEMS:
            items.append({"_truncated_items": len(v) - MAX_ITEMS})
        return items
    return v

for call in sorted((root / "calls").glob("*.json")):
    req = json.loads(call.read_text())
    res = send("tools/call", {"name": req["tool"], "arguments": req["arguments"]})["result"]
    body = res.get("structuredContent")
    if body is None:
        body = {"isError": res.get("isError"), "content": [json.loads(c["text"]) if c["text"].startswith("{") else c["text"] for c in res["content"]]}
    (root / "responses" / call.name).write_text(json.dumps(scrub(body), indent=2, ensure_ascii=False) + "\n")
    print("captured", call.name)
proc.stdin.close()
proc.wait()
