"""The team-lead skill's checkable rules, graded on real lead sessions.

The skill lives in the vault (~/.claude/skills/team-lead/SKILL.md) and
changes often; these checks encode its rules as of 2026-09-29. Each check
counts instances (one per SendMessage, per `herdr agent start`, per merge...)
as passes and failures, or returns None when the session has none.
"""
import json
import re
import sys
import os

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "harness"))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from transcript import flag_value, has_flag  # noqa: E402
from history import real_segments  # noqa: E402

NAME = "team-lead"
SPLIT = None
LEVELS = r"(?:low|medium|high|xhigh|max|ultra)"
# A start's arguments run to the end of its shell segment, so two starts
# chained on one line are two.
START = re.compile(r"herdr agent start\s+(\S+)([^\n;&|]*)")
NAME_RE = re.compile(r"^[a-z][a-z0-9_-]{0,31}$")


def select(session):
    for tool in session.t.tools:
        if tool.name == "Skill" and re.search(r"team-lead|herdr-team", str(tool.input.get("skill", ""))):
            return True
        if tool.name == "Bash" and START.search(tool.cmd):
            return True
    return False


def _to_worker(tool):
    """Whom a SendMessage goes to, or None when that is the parent session (main) or a socket address."""
    to = str(tool.input.get("to", ""))
    if to == "main" or to.startswith("uds:") or ("[" in to and "uds" in to):
        return None
    return to


def _sends(session):
    """SendMessage calls to workers."""
    for tool in session.t.tools:
        if tool.name != "SendMessage":
            continue
        to = _to_worker(tool)
        if to is not None:
            yield tool, to, tool.input.get("message")


def _count(items):
    items = list(items)
    if not items:
        return None
    return sum(1 for x in items if x), sum(1 for x in items if not x)


def notify_when_idle(session):
    """Every message to a worker carries notify_when_idle."""
    return _count(bool(t.input.get("notify_when_idle")) for t, _, _ in _sends(session))


def brief_ends_with_reporting(session):
    """Every brief (the first message to a worker, or the first after a /clear) ends with the Reporting block."""
    briefed = set()
    cleared = set()
    out = []
    for tool in session.t.tools:
        if tool.name == "Bash":
            for m in re.finditer(r'herdr agent prompt\s+(\S+)\s+["\']/clear', tool.cmd):
                cleared.add(m.group(1))
        if tool.name != "SendMessage":
            continue
        to = _to_worker(tool)
        msg = tool.input.get("message")
        if not isinstance(msg, str) or to is None:
            continue
        if to in briefed and to not in cleared:
            continue
        briefed.add(to)
        cleared.discard(to)
        i = msg.rfind("Reporting. I am")
        out.append(i >= 0 and "\n\n" not in msg[i:].strip())
    return _count(out)


def agent_start_model_effort(session):
    """Every `herdr agent start` passes --model and --effort to Claude."""
    out = []
    for tool in session.t.tools:
        for m in START.finditer(tool.cmd):
            rest = m.group(2)
            out.append(has_flag(rest, "--model") and has_flag(rest, "--effort"))
    return _count(out)


def agent_start_matches_table(session):
    """Workers start at a build effort of medium, on opus or claude-sonnet-5-5 (the table's rows)."""
    out = []
    for tool in session.t.tools:
        for m in START.finditer(tool.cmd):
            model, effort = flag_value(m.group(2), "--model"), flag_value(m.group(2), "--effort")
            if model is None or effort is None:
                continue
            out.append(model in ("opus", "claude-sonnet-5-5") and effort == "medium")
    return _count(out)


def agent_start_no_permission_mode(session):
    """No permission mode on agent start: workers inherit auto."""
    return _count("--permission-mode" not in m.group(2) and "--dangerously" not in m.group(2)
                  for tool in session.t.tools for m in START.finditer(tool.cmd))


def one_start_per_call(session):
    """One `herdr agent start` per Bash call."""
    return _count(len(START.findall(tool.cmd)) <= 1 for tool in session.t.tools if START.search(tool.cmd))


def worker_names(session):
    """Worker names: 1 to 3 kebab-case words, no type prefix, no number."""
    out = []
    for tool in session.t.tools:
        for m in START.finditer(tool.cmd):
            name = m.group(1)
            out.append(bool(NAME_RE.match(name)) and not re.match(r"(feat|fix|chore|docs|test)[-_]", name)
                       and not re.search(r"\d", name) and name.count("-") <= 2)
    return _count(out)


def effort_raised_on_built(session):
    """After a worker reports `built`, the lead sends it /effort before its next message to that worker."""
    out = []
    for at, text in session.incoming:
        m = re.search(r'from-name="([^"]+)"', text)
        if not m or not re.search(r"\bbuilt\b", text[:400], re.I):
            continue
        worker = m.group(1)
        raised = None
        for tool in session.t.tools[at:]:
            if tool.name == "Bash" and re.search(r'herdr agent prompt\s+%s\s+["\']/effort' % re.escape(worker), tool.cmd):
                raised = True
                break
            if tool.name == "SendMessage" and tool.input.get("to") == worker:
                raised = False
                break
        if raised is not None:
            out.append(raised)
    return _count(out)


def code_review_level(session):
    """Every /code-review the lead runs or asks for names its level."""
    out = []
    for tool in session.t.tools:
        if tool.name == "Skill" and tool.input.get("skill") == "code-review":
            # The level may follow flags: "--fix high".
            out.append(bool(re.match(r"(?:--\S+\s+)*" + LEVELS + r"\b", str(tool.input.get("args", "")).strip())))
    for tool, _, msg in _sends(session):
        if isinstance(msg, str):
            for m in re.finditer(r"/code-review\b`?(?=(.{0,28}))", msg):
                after = m.group(1)
                # "what /code-review found" (the Reporting block's field) names
                # the review; it does not ask for one.
                if re.match(r"\s*(found|finds|find|ran|runs|result|results|output|'s|reported)\b", after):
                    continue
                # The level may follow the flags in words: "/code-review --fix` at medium".
                out.append(bool(re.match(r"\s+`?" + LEVELS + r"\b", after)
                                or re.match(r"\s+--fix`?,?\s+(at|with)\s+`?" + LEVELS + r"\b", after)))
    return _count(out)


def merge_after_checks(session):
    """A merge follows `gh pr checks` in the same turn (on the same PR when both name one)."""
    out = []
    tools = session.t.tools
    for i, tool in enumerate(tools):
        for m in re.finditer(r"gh pr merge\s+(\d+)?", tool.cmd):
            pr = m.group(1)
            turn = session.turn_of[i] if i < len(session.turn_of) else None
            ok = False
            for j in range(i, -1, -1):
                if j < len(session.turn_of) and session.turn_of[j] != turn:
                    break
                for c in re.finditer(r"gh pr checks\s+(\d+)?", tools[j].cmd):
                    if j < i or c.start() < m.start():
                        if not pr or not c.group(1) or c.group(1) == pr:
                            ok = True
            out.append(ok)
    return _count(out)


def cases_open_label_worker(session):
    """Every case the lead opens carries --label and --worker."""
    return _count(has_flag(a, "--label") and has_flag(a, "--worker")
                  for tool in session.t.tools for s, a in real_segments(tool) if s == "open")


def one_label_wait(session):
    """At most one `cases wait --label` runs at a time."""
    waits = []
    for tool in session.t.tools:
        for s, a in real_segments(tool):
            if s == "wait" and has_flag(a, "--label") and tool.bg:
                end = tool.done_at if tool.done_at is not None else len(session.t.tools) + 1
                waits.append((tool.index, end))
    if not waits:
        return None
    out = []
    for k, (start, _) in enumerate(waits):
        out.append(not any(s2 < start < e2 for s2, e2 in waits[:k]))
    return _count(out)


def lead_pane_marked(session):
    """The lead marks its own pane role=lead before starting workers (a rule added in late September)."""
    starts = [t.index for t in session.t.tools if START.search(t.cmd)]
    if not starts:
        return None
    marked = any("report-metadata" in t.cmd and "role=lead" in t.cmd for t in session.t.tools[:starts[0]])
    return _count([marked])


RULES = [
    ("notify_when_idle", "every message to a worker has notify_when_idle", notify_when_idle),
    ("brief_ends_with_reporting", "every brief ends with the Reporting block", brief_ends_with_reporting),
    ("agent_start_model_effort", "agent start passes --model and --effort", agent_start_model_effort),
    ("agent_start_matches_table", "build effort medium, model opus or claude-sonnet-5-5", agent_start_matches_table),
    ("agent_start_no_permission_mode", "no permission mode on agent start", agent_start_no_permission_mode),
    ("one_start_per_call", "one agent start per Bash call", one_start_per_call),
    ("worker_names", "worker names follow the Names rules", worker_names),
    ("effort_raised_on_built", "/effort sent on `built` before the verify message", effort_raised_on_built),
    ("code_review_level", "/code-review always names a level", code_review_level),
    ("merge_after_checks", "gh pr merge follows gh pr checks in the same turn", merge_after_checks),
    ("cases_open_label_worker", "cases open carries --label and --worker", cases_open_label_worker),
    ("one_label_wait", "at most one label wait at a time", one_label_wait),
    ("lead_pane_marked", "the lead marks its pane role=lead before starting workers", lead_pane_marked),
]

JUDGEMENT = [
    ("never route a blocked action", "Whether the lead ran a worker's blocked command on its own initiative "
     "depends on whether the user approved it, which may be in chat, a case answer or an earlier session."),
    ("model fits the work", "Whether work was well-scoped (Sonnet) or not (opus) is a reading of the task."),
    ("verify effort matches the table", "The verify level depends on the work type and on whether an earlier "
     "verify missed something; a program sees the /effort but not the reason."),
    ("brief content", "Whether a brief stands alone, gives why, verified facts and the write policy, and says "
     "where authority comes from, is a reading of prose."),
    ("do not do the workers' work", "Telling the lead's own edits apart from bookkeeping needs the task."),
    ("close on evidence", "Whether a close or merge rested on checked evidence rather than the report."),
    ("relay in your own words, review board", "Rollup quality is prose."),
    ("act on a stop only when owed", "Whether a nudge was due depends on what the worker owed at that moment."),
    ("reuse or clear a worker", "Whether the next task built on the last one is a judgement of topic."),
    ("teardown", "Whether every tab, worktree and case was dealt with spans the whole run and later sessions."),
]
