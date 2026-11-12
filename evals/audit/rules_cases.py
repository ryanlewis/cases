"""The cases skill's rules, graded on real sessions.

It reuses the eval's graders (evals/cases-skill/graders.py) for every rule a
transcript alone can settle. The rules that read the eval's scratch store,
its human script or its task's expectations are skipped: in real use there
is no copy of the store as the session left it, and no task says what the
right number of cases was. `cases` calls that name another store
(`--store`, `CASES_STORE=`) are CLI testing, not use, and are dropped.
"""
import os
import re
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "cases-skill"))
import graders  # noqa: E402
from history import real_segments  # noqa: E402
from transcript import chain_open_waits  # noqa: E402

NAME = "cases"
# #78 (25f449f) rewrote the skill: one background command per decision.
SPLIT = ("#78", "2026-09-28T18:13:16Z")

TRANSCRIPT_RULES = [
    "no_human_writes", "wait_scoped", "revision_on_note_close", "wait_in_background",
    "wait_since", "one_pickup_wait_per_case", "open_chained", "close_chained", "wait_output_read_once",
    "exit_3_4_handled", "status_beside_first_open",
]
SKIPPED = {
    "closed_all_pickedup": "needs the store as the session left it",
    "no_duplicate": "compares case titles in the store",
    "opens_as_expected": "needs a task's expected number of cases",
    "no_secret": "needs a task's planted secret",
    "withdrawn_respected": "finds human withdrawals in the store",
    "parked_handled": "needs the scripted human's log of parks and resumes",
    "message_handled": "finds messages in the store",
    "listed_on_resume": "needs a task that says the session is a resume",
    "noted_unclear": "needs a task that says which answer was unclear",
    "outcome": "task-specific checks on the workspace",
}
USE = {"open", "wait", "close", "pickup", "note", "withdraw", "amend"}


def _real_use(tool):
    """Drops a tool's cases segments that name another store."""
    tool.segments = real_segments(tool)
    tool.subs = [s for s, _ in tool.segments]


class HistoryRun:
    """What the graders read from a Run, from a real session alone."""

    def __init__(self, session):
        self.t = session.t
        self.store = []
        self.human = []
        self.expect = {}
        self.agent_cases = []
        self.task = {}

    def segs(self):
        for tool in self.t.tools:
            for sub, args in tool.segments:
                yield tool, sub, args

    def counted_cases(self):
        return []

    def opened_by(self, tool):
        return []


MENTION = "cases wait"


def classify(session):
    """Why a transcript that mentions `cases wait` is, or is not, real use."""
    raw = [t for t in session.t.tools if t.name == "Bash"
           and re.search(r"(^|[\s;&|(/])cases\s+\w", getattr(t, "raw_cmd", t.cmd))]
    if not raw:
        return "text only: no cases command run (skill text, docs, prompts, output)"
    if not any(t.segments for t in session.t.tools):
        return "no cases call outside heredocs, quoted strings or --help (CLI development)"
    if not select(session):
        if any(t.segments for t in session.t.tools):
            return "read-only calls only (list, show, status, skill, config)"
        return "CLI testing: every call names another store"
    return "real use"


def select(session):
    for tool in session.t.tools:
        _real_use(tool)
    # An open --wait waits on the case it made. With no store to match it
    # to, the wait names it as "$opened", as the graders do an open they
    # cannot match: the rules read that it is scoped, has --since and runs
    # where the open does. one_pickup_wait_per_case cannot tell which case
    # "$opened" is, so it never finds this wait clashing with another.
    chain_open_waits(session.t.tools, lambda tool, k: "$opened")
    return any(set(tool.subs) & USE for tool in session.t.tools)


def _rule(name):
    fn = dict(graders.RULES)[name]

    def check(session):
        v = fn(HistoryRun(session))
        if v["result"] == "pass":
            return 1, 0
        if v["result"] == "fail":
            return 0, 1
        return None
    return check


def never_sweep_prune_or_real_store(session):
    """The graders' no_forbidden, cut to what is never allowed: sweep, prune,
    and touching the real store file. serve, service, skill install and inbox
    are the human's 'unless asked', which history cannot tell (the sessions
    that build those commands run them all the time)."""
    bad = [s for tool in session.t.tools for s in tool.subs if s in ("sweep", "prune")]
    bad += [t.cmd for t in session.t.tools
            if re.search(r"(sqlite3|\brm\b|\bmv\b|\bcp\b)[^\n;&|]*\.local/share/cases/cases\.db", t.cmd)]
    return (0, 1) if bad else (1, 0)


RULES = ([(name, "", _rule(name)) for name in TRANSCRIPT_RULES]
         + [("never_sweep_prune_or_real_store", "", never_sweep_prune_or_real_store)])
SKIPPED["no_forbidden"] = ("graded as never_sweep_prune_or_real_store: serve, service, skill install and inbox "
                           "are allowed when asked, which history cannot tell")
JUDGEMENT = [("serve, service, skill install, inbox unasked", "whether the human asked is in the conversation's "
              "prose, and the sessions that build those commands run them by design")]


# Cost as the cost study counted it: prices per million tokens (Opus 5.5).
P = dict(inp=4, w5=5, w1h=8, read=0.2, out=20)


def _cost_of(u):
    cw = u.get("cache_creation") or {}
    w1 = cw.get("ephemeral_1h_input_tokens", 0)
    w5 = cw.get("ephemeral_5m_input_tokens", 0) if cw else u.get("cache_creation_input_tokens", 0)
    return (u.get("input_tokens", 0) * P["inp"] + w5 * P["w5"] + w1 * P["w1h"]
            + u.get("cache_read_input_tokens", 0) * P["read"] + u.get("output_tokens", 0) * P["out"]) / 1e6


def cost(session):
    """Per session: decisions (cases opened, plus messages taken), the API calls
    that made only cases bookkeeping (`Transcript.call_classes`: an open is the
    ask; the rest are extra), and what those cases-only calls cost."""
    t = session.t
    opens = sum(s == "open" for tool in t.tools for s in tool.subs)
    messages = sum(len(re.findall(r'"kind":\s*"message"', tool.text())) for tool in t.tools if "wait" in tool.subs)
    # The same calls call_classes counts as ask or extra, reads of a wait's output included.
    cls = t.call_classes()
    dollars = sum(_cost_of(t.calls[t.call_order[ci]]) for ci in cls["calls"] if ci < len(t.call_order))
    return {"decisions": opens + messages, "ask": cls["ask"], "extra": cls["extra"], "cases_only_cost": dollars}
