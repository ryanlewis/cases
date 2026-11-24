#!/usr/bin/env python3
"""Grades real Claude Code sessions against a skill's checkable rules, offline.

    python3 evals/audit/audit.py --rules evals/audit/rules_cases.py \\
        [--rules evals/audit/rules_team_lead.py] [--glob '~/.claude/projects/*/*.jsonl'] --out REPORT.md

It only reads transcripts and writes one report. A rules module names itself
(NAME), picks its sessions (select(session)), and lists its checks (RULES:
name, description, check(session) returning (passes, failures) or None).
It may also give SPLIT (label, ISO time) to split sessions before and after a
change, SKIPPED (rule: why) and JUDGEMENT ((rule, why) for rules a program
cannot check), and cost(session). The report holds counts, rule names and
session ids only, never transcript text.
"""
import argparse
import datetime
import glob
import importlib.util
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "harness"))
from history import Session  # noqa: E402
import math  # noqa: E402

from stats import fmt_rate, wilson  # noqa: E402

# The eval's own workspaces (Claude Code leaves an empty project dir for each,
# the runner's --no-session-persistence keeps the transcript out) and the probe
# sessions run while the eval was built are not real use.
EVAL_PROJECTS = re.compile(r"cases-eval-|skill-evals-.*-scratchpad-")


def load_module(path):
    spec = importlib.util.spec_from_file_location(os.path.basename(path)[:-3], path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def grade(mod, sessions):
    """Per session: its period and each rule's (passes, failures) or None."""
    rows = []
    for s in sessions:
        if not mod.select(s):
            continue
        split = getattr(mod, "SPLIT", None)
        period = "all" if not split else ("before " + split[0] if s.start < split[1] else "after " + split[0])
        results = {}
        for name, _, check in mod.RULES:
            try:
                results[name] = check(s)
            except Exception as e:  # a check that cannot read a session is reported, not scored
                results[name] = ("error", f"{type(e).__name__}: {e}")
        cost = mod.cost(s) if hasattr(mod, "cost") else None
        rows.append({"id": s.id, "start": s.start, "period": period, "results": results, "cost": cost})
    return rows


def periods_of(mod):
    split = getattr(mod, "SPLIT", None)
    return ["before " + split[0], "after " + split[0]] if split else ["all"]


def newcombe(p1, n1, p2, n2):
    """The difference of two pass rates (second minus first) with Newcombe's
    hybrid score interval, and whether it lies beyond noise."""
    if not n1 or not n2:
        return "n/a", "n/a"
    a, l1, u1 = wilson(p1, n1)
    b, l2, u2 = wilson(p2, n2)
    d = b - a
    lo = d - math.sqrt((b - l2) ** 2 + (u1 - a) ** 2)
    hi = d + math.sqrt((u2 - b) ** 2 + (a - l1) ** 2)
    verdict = "inside noise" if lo <= 0 <= hi else ("better" if lo > 0 else "worse")
    return f"{100 * d:+.0f} pts [{100 * lo:+.0f}, {100 * hi:+.0f}]", verdict


def classes(mod, paths):
    """How the transcripts that mention the module's text divide: the sampling behind the tables."""
    mention = getattr(mod, "MENTION", None)
    if not mention or not hasattr(mod, "classify"):
        return []
    counts = {}
    for p in paths:
        with open(p, encoding="utf-8", errors="replace") as f:
            if mention not in f.read():
                continue
        if EVAL_PROJECTS.search(os.path.basename(os.path.dirname(p))):
            k = "the eval's own runs and probes"
        else:
            try:
                k = mod.classify(Session(p))
            except Exception:
                k = "unreadable"
        counts[k] = counts.get(k, 0) + 1
    lines = [f"### Sampling", "", f"Transcripts that contain the text \"{mention}\": {sum(counts.values())}.", "",
             "| class | transcripts |", "| --- | --- |"]
    lines += [f"| {k} | {n} |" for k, n in sorted(counts.items(), key=lambda kv: -kv[1])]
    return lines + [""]


def section(mod, rows):
    periods = periods_of(mod)
    lines = [f"## {mod.NAME}", ""]
    if not rows:
        return lines + ["No sessions selected.", ""]
    starts = sorted(r["start"] for r in rows)
    lines.append(f"{len(rows)} sessions, {starts[0][:10]} to {starts[-1][:10]}: "
                 + ", ".join(f"{p} {sum(r['period'] == p for r in rows)}" for p in periods) + ".")
    split = getattr(mod, "SPLIT", None)
    if split:
        lines.append(f"Split at {split[1]} ({split[0]}), by each session's first event.")
    lines.append("")
    lines.append("Sessions that pass the rule wholly, of those it applies to, with a 95% Wilson interval; "
                 "instances passed / checked in brackets.")
    lines.append("")
    dated = not split
    if dated:
        lines += ["With no record of when each rule was written, \"first pass\" is the first session that "
                  "passed the rule; a late date suggests the rule is newer than the sessions before it, whose "
                  "failures then say little."]
        lines.append("")
    two = len(periods) == 2
    if two:
        lines += ["The difference is after minus before, in sessions passing, with a 95% Newcombe interval "
                  "(from the two Wilson intervals); \"inside noise\" means that interval includes zero.", ""]
    head = ["rule", *periods] + (["first pass", "since first pass"] if dated else []) + \
        (["difference [95%]", "beyond noise?"] if two else [])
    lines.append("| " + " | ".join(head) + " |")
    lines.append("|" + " --- |" * len(head))
    for name, desc, _ in mod.RULES:
        cells = []
        rates = []
        for p in periods:
            vals = [r["results"][name] for r in rows if r["period"] == p]
            vals = [v for v in vals if v is not None and v[0] != "error"]
            ok = sum(1 for v in vals if v[1] == 0)
            rates.append((ok, len(vals)))
            inst_p = sum(v[0] for v in vals)
            inst_n = sum(v[0] + v[1] for v in vals)
            cells.append(f"{fmt_rate(ok, len(vals))} ({inst_p}/{inst_n})" if vals else "n/a")
        if two:
            cells += list(newcombe(*rates[0], *rates[1]))
        if dated:
            vals = sorted((r["start"], r["results"][name]) for r in rows
                          if r["results"][name] is not None and r["results"][name][0] != "error")
            first = next((st for st, v in vals if v[1] == 0), None)
            since = [v for st, v in vals if first and st >= first]
            cells += [first[:10] if first else "never",
                      fmt_rate(sum(1 for v in since if v[1] == 0), len(since)) if since else "n/a"]
        lines.append(f"| {name} | " + " | ".join(cells) + " |")
    errors = sorted({(name, r["id"]) for r in rows for name, v in r["results"].items()
                     if v is not None and v[0] == "error"})
    if errors:
        lines += ["", "Checks that could not read a session: "
                  + ", ".join(f"{n} on {i}" for n, i in errors) + "."]
    if any(r["cost"] for r in rows):
        lines += ["", "### Cost per decision", "",
                  "A decision is a case opened or a message taken. Extra calls are API calls that made only "
                  "`cases` bookkeeping other than an open: every command in them is a `cases` call or shell glue, so work "
                  "chained with a close does not count. Before 2026-11-09 any call with a `cases` call in it counted, "
                  "so older reports are not comparable. Their cost is at Opus 5.5 prices.",
                  "", "| period | sessions | decisions | extra calls / decision | ask calls / decision | "
                  "$ of cases-only calls / decision |", "| --- | --- | --- | --- | --- | --- |"]
        for p in periods:
            cs = [r["cost"] for r in rows if r["period"] == p and r["cost"]]
            d = sum(c["decisions"] for c in cs)
            if not d:
                lines.append(f"| {p} | {len(cs)} | 0 | n/a | n/a | n/a |")
                continue
            lines.append(f"| {p} | {len(cs)} | {d} | {sum(c['extra'] for c in cs) / d:.2f} | "
                         f"{sum(c['ask'] for c in cs) / d:.2f} | {sum(c['cases_only_cost'] for c in cs) / d:.3f} |")
    skipped = getattr(mod, "SKIPPED", {})
    if skipped:
        lines += ["", "### Rules not graded on history", ""]
        lines += [f"- {k}: {v}" for k, v in skipped.items()]
    judgement = getattr(mod, "JUDGEMENT", [])
    if judgement:
        lines += ["", "### Rules that need judgement", ""]
        lines += [f"- {k}: {v}" for k, v in judgement]
    lines += ["", "### Failing sessions by rule", ""]
    for name, _, _ in mod.RULES:
        bad = [r for r in rows if r["results"][name] and r["results"][name][0] != "error" and r["results"][name][1]]
        if bad:
            lines.append(f"- {name}: " + ", ".join(f"{r['id'][:8]} ({r['period']})" for r in bad))
    lines.append("")
    return lines


def main(argv=None):
    ap = argparse.ArgumentParser()
    ap.add_argument("--rules", action="append", required=True, help="a rules module; repeatable")
    ap.add_argument("--glob", default="~/.claude/projects/*/*.jsonl")
    ap.add_argument("--out", required=True, help="the report to write (markdown)")
    a = ap.parse_args(argv)
    mods = [load_module(p) for p in a.rules]
    paths = sorted(p for p in glob.glob(os.path.expanduser(a.glob))
                   if not EVAL_PROJECTS.search(os.path.basename(os.path.dirname(p))))
    skipped_eval = len(glob.glob(os.path.expanduser(a.glob))) - len(paths)
    sessions = []
    unreadable = 0
    for p in paths:
        try:
            sessions.append(Session(p))
        except Exception:
            unreadable += 1
    lines = [f"# Skill audit of real sessions, {datetime.date.today().isoformat()}", "",
             f"Read {len(sessions)} transcripts ({unreadable} unreadable); left out {skipped_eval} from the eval's "
             "own runs and scratch probes. Built by `evals/audit/audit.py` from counts, rule names and session "
             "ids only.", ""]
    all_paths = sorted(glob.glob(os.path.expanduser(a.glob)))
    for mod in mods:
        sec = section(mod, grade(mod, sessions))
        lines += sec[:2] + classes(mod, all_paths) + sec[2:]
    with open(os.path.expanduser(a.out), "w", encoding="utf-8") as f:
        f.write("\n".join(lines) + "\n")
    print(os.path.expanduser(a.out))


if __name__ == "__main__":
    main()
