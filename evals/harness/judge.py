"""An LLM judge for what a program cannot check.

It asks a small model whether each of a few claims holds for an item, and
wants true, false or null (the claim does not apply) with a one-line reason
for each: checkable claims, never a score out of 10. Items go in batches, one
`claude -p` per batch, with no tools, so the judge costs a few sessions per
suite run rather than one per run.
"""
import json
import os
import re
import subprocess

from runner import base_env

PROMPT = """You grade transcripts of an AI agent that asks a human questions through an inbox of "cases".
For each item below, decide each claim: true, false, or null when the claim does not apply to the item.
Be strict and literal. Judge only from what the item shows.

Claims:
{claims}

Reply with JSON only, no prose and no code fence: an object mapping each item's "key" to an object that maps
each claim id to {{"v": true|false|null, "why": "one short line"}}.

Items:
{items}
"""


MODEL = "claude-haiku-4-5"
BATCH = 20


def judge(items, claims, *, claude, model=MODEL, batch=BATCH, log_dir=None):
    """items: list of {"key": str, ...}. claims: list of (id, text).
    Returns key -> claim id -> {"v", "why"}, and the number of sessions used."""
    verdicts = {}
    sessions = 0
    claim_text = "\n".join(f"- {cid}: {text}" for cid, text in claims)
    env = base_env("HOME", "USER", "LOGNAME", "LANG", "PATH")
    for start in range(0, len(items), batch):
        chunk = items[start:start + batch]
        prompt = PROMPT.format(claims=claim_text, items=json.dumps(chunk, indent=1))
        sessions += 1
        text = ""
        try:
            p = subprocess.run([claude, "-p", "--model", model, "--tools", "", "--output-format", "json",
                                "--setting-sources", "project,local", "--no-session-persistence"],
                               input=prompt, capture_output=True, text=True, env=env, timeout=600,
                               cwd=log_dir or None)
            text = json.loads(p.stdout).get("result", "")
            m = re.search(r"\{.*\}", text, re.S)
            got = json.loads(m.group(0)) if m else {}
        except (json.JSONDecodeError, AttributeError, subprocess.TimeoutExpired) as e:
            got = {}
            if isinstance(e, subprocess.TimeoutExpired):
                text = "timed out"
        if not isinstance(got, dict):
            got = {}
        if log_dir:
            with open(os.path.join(log_dir, "judge.log"), "a", encoding="utf-8") as f:
                f.write(json.dumps({"batch": start, "raw": text[:20000]}) + "\n")
        for it in chunk:
            # Only well-formed verdicts, so a stray shape cannot break the report.
            v = got.get(it["key"])
            verdicts[it["key"]] = {c: x for c, x in v.items() if isinstance(x, dict)} \
                if isinstance(v, dict) else {}
    return verdicts, sessions
