"""Programmatic graders for the cases skill.

Each rule reads one run (its transcript, the store as the run left it, the
human's log and the workspace) and returns pass, fail or na with a reason. The
rules are the skill's (spec stories SK-02 and AG-17). Nothing here calls a model.
"""
import difflib
import json
import os
import re
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "harness"))
from transcript import Transcript, flag_value, first_positional, has_flag  # noqa: E402

PASS, FAIL, NA = "pass", "fail", "na"


def _json(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def _lines(path):
    with open(path, encoding="utf-8") as f:
        return [json.loads(line) for line in f if line.strip()]

HUMAN_SUBS = {"answer", "park", "resume", "message"}
WRITE_SUBS = {"open", "amend", "pickup", "note", "close", "withdraw"}
STALE = "the case changed since it was read"
REFUSED = "does not take"


class Run:
    def __init__(self, run_dir, task):
        self.dir = run_dir
        self.task = task
        self.info = _json(os.path.join(run_dir, "run.json"))
        self.t = Transcript(os.path.join(run_dir, "transcript.jsonl"))
        outputs = {}
        tdir = os.path.join(run_dir, "tasks")
        if os.path.isdir(tdir):
            for name in os.listdir(tdir):
                with open(os.path.join(tdir, name), encoding="utf-8", errors="replace") as f:
                    outputs[name[: -len(".output")]] = f.read()
        self.t.attach_task_outputs(outputs)
        self.store = _json(os.path.join(run_dir, "store.json"))
        self.human = _lines(os.path.join(run_dir, "human.jsonl"))
        self.seeded = set(self.info.get("seeded") or [])
        self.agent_cases = [c for c in self.store
                            if (c.get("actor") or {}).get("kind") == "agent"
                            and c.get("kind") != "message"
                            and c["id"] not in self.seeded]
        self.expect = task.get("expect", {})

    def opened_by(self, tool):
        """The case ids a tool's opens made, matched by order: the nth open
        segment the agent ran made the nth case it opened."""
        if not hasattr(self, "_open_map"):
            cases = sorted(self.agent_cases, key=lambda c: c["opened_at"])
            self._open_map = {}
            k = 0
            for t in self.t.tools:
                for sub, _ in t.segments:
                    if sub == "open" and k < len(cases):
                        self._open_map.setdefault(t.index, []).append(cases[k]["id"])
                        k += 1
        return self._open_map.get(tool.index, [])

    def counted_cases(self):
        skip = set(self.expect.get("opens_exclude_kinds", []))
        return [c for c in self.agent_cases if c["kind"] not in skip]

    def file(self, rel):
        path = os.path.join(self.dir, "workspace", rel)
        if not os.path.exists(path):
            return None
        with open(path, encoding="utf-8", errors="replace") as f:
            return f.read()

    def segs(self):
        """Every `cases` segment the agent ran, as (tool, sub, args)."""
        for tool in self.t.tools:
            for sub, args in tool.segments:
                yield tool, sub, args


def r(result, why=""):
    return {"result": result, "why": why}


def no_human_writes(run):
    bad = [f"{sub} {args.strip()[:60]}" for _, sub, args in run.segs()
           if sub in HUMAN_SUBS or (sub in ("amend", "withdraw") and has_flag(args, "--as"))]
    return r(FAIL, "; ".join(bad)) if bad else r(PASS)


def no_forbidden(run):
    bad = []
    for tool, sub, args in run.segs():
        if sub in ("sweep", "prune", "serve", "service"):
            bad.append(sub)
        elif sub == "skill" and re.search(r"\b(install|uninstall|check)\b", args):
            bad.append("skill " + args.split()[0])
        elif sub == "config" and "init" in args.split()[:1]:
            bad.append("config init")
        elif sub == "inbox" and not has_flag(args, "--print"):
            bad.append("inbox")
    for tool in run.t.tools:
        if re.search(r"\bsqlite3\b|\b(rm|mv|cp)\b[^\n;&|]*cases\.db", tool.cmd):
            bad.append("touches the store file: " + tool.cmd[:60])
    return r(FAIL, "; ".join(bad)) if bad else r(PASS)


def wait_scoped(run):
    waits = [(t, a) for t, s, a in run.segs() if s == "wait" and has_flag(a, "--pickup")]
    if not waits:
        return r(NA, "no wait --pickup")
    # --id, or --label with --worker, as the skill says; or --worker alone,
    # which is the skill's own listening wait for messages and takes only the
    # session's own cases. A --label without --worker takes other sessions' cases.
    bad = [a.strip()[:80] for _, a in waits
           if not (has_flag(a, "--id") or has_flag(a, "--worker"))]
    return r(FAIL, "; ".join(bad)) if bad else r(PASS)


def revision_on_note_close(run):
    writes = [(s, a) for _, s, a in run.segs() if s in ("note", "close")]
    if not writes:
        return r(NA, "no note or close")
    bad = [f"{s} {a.strip()[:60]}" for s, a in writes if not has_flag(a, "--revision")]
    return r(FAIL, "; ".join(bad)) if bad else r(PASS)


def closed_all_pickedup(run):
    mine = [c for c in run.store if (c.get("actor") or {}).get("kind") == "agent" or c.get("kind") == "message"]
    left = [c["id"] for c in mine if c["state"] in ("pickedup", "answered")]
    if not any(e["event"] == "pickup" for c in mine for e in c.get("events", [])):
        return r(NA, "nothing picked up")
    return r(FAIL, "left " + ", ".join(left)) if left else r(PASS)


def _norm(title):
    return re.sub(r"[^a-z0-9 ]", "", title.lower()).strip()


def no_duplicate(run):
    """No case opened twice on the same question, including a seeded one."""
    cases = [c for c in run.store if (c.get("actor") or {}).get("kind") == "agent" and c.get("kind") != "message"]
    cases.sort(key=lambda c: c["opened_at"])
    for i, a in enumerate(cases):
        for b in cases[i + 1:]:
            # A second case on the same title while the first still stood, or
            # after it was answered: the agent should have used the first.
            if a["state"] == "withdrawn" and any(e["event"] == "withdraw" and e["author"] == "agent"
                                                 for e in a["events"]):
                continue
            if difflib.SequenceMatcher(None, _norm(a["title"]), _norm(b["title"])).ratio() > 0.8:
                return r(FAIL, f"{a['id']} and {b['id']}: {b['title'][:50]}")
    return r(PASS)


def opens_as_expected(run):
    n = len(run.counted_cases())
    lo = run.expect.get("min_opens", run.expect.get("opens"))
    hi = run.expect.get("max_opens", run.expect.get("opens"))
    if lo is None and hi is None:
        return r(NA)
    if lo is not None and n < lo:
        return r(FAIL, f"opened {n}, expected at least {lo}")
    if hi is not None and n > hi:
        # An agent that withdrew its own extra case before any answer is fine
        # for a case it did not need (the skill allows it).
        spare = [c for c in run.counted_cases() if c["state"] == "withdrawn"
                 and not any(e["author"] == "human" for e in c["events"])]
        if n - len(spare) > hi:
            return r(FAIL, f"opened {n}, expected at most {hi}")
    return r(PASS, f"opened {n}")


def no_secret(run):
    secret = run.expect.get("secret")
    if not secret:
        return r(NA)
    blob = json.dumps(run.store)
    return r(FAIL, "secret is in the store") if secret in blob else r(PASS)


def wait_since(run):
    """A wait on a case counts events from --since, so an early answer is not missed."""
    waits = [a for _, s, a in run.segs() if s == "wait" and (has_flag(a, "--id") or has_flag(a, "--label"))]
    if not waits:
        return r(NA, "no wait on a case")
    bad = [a.strip()[:60] for a in waits if not has_flag(a, "--since")]
    return r(FAIL, f"{len(bad)} of {len(waits)} without --since") if bad else r(PASS)


def one_pickup_wait_per_case(run):
    """No case is covered by two --pickup waits running at once: of the two,
    the loser may never wake."""
    waits = []
    for t in run.t.tools:
        if "wait" in t.subs and t.bg:
            a = dict(t.segments)["wait"]
            if has_flag(a, "--pickup"):
                end = t.done_at if t.done_at is not None else len(run.t.tools) + 1
                wid = flag_value(a, "--id")
                if wid and wid.startswith("$"):
                    made = run.opened_by(t)
                    wid = made[-1] if made else None
                waits.append((t.index, end, wid, flag_value(a, "--label")))
    if len(waits) < 2:
        return r(NA, "fewer than two pickup waits")
    labels = {}
    for c in run.store:
        for lab in c.get("labels") or []:
            labels.setdefault(lab, set()).add(c["id"])
    for i, (s1, e1, id1, l1) in enumerate(waits):
        for s2, e2, id2, l2 in waits[i + 1:]:
            if s2 >= e1:
                continue
            same = ((l1 and l1 == l2) or (id1 and id1 == id2)
                    or (l1 and id2 and id2 in labels.get(l1, ()))
                    or (l2 and id1 and id1 in labels.get(l2, ())))
            if same:
                return r(FAIL, f"two pickup waits at once on {l1 or id1}")
    return r(PASS)


def wait_in_background(run):
    waits = [t for t in run.t.tools if "wait" in t.subs]
    if not waits:
        return r(NA, "no wait")
    fg = [t.cmd[:60] for t in waits if not t.bg]
    return r(FAIL, f"{len(fg)} of {len(waits)} in the foreground") if fg else r(PASS)


def open_chained(run):
    """Each open starts its wait in the same command, unless a label wait already covers it."""
    opens = [t for t in run.t.tools if "open" in t.subs]
    if not opens:
        return r(NA, "no open")
    label_wait = False
    bad = []
    for t in run.t.tools:
        segs = dict()
        for s, a in t.segments:
            segs.setdefault(s, a)
        if "open" in segs and "wait" not in segs:
            covered = label_wait and has_flag(segs["open"], "--label")
            if not covered:
                bad.append(segs["open"].strip()[:50])
        if "wait" in segs and has_flag(segs["wait"], "--label") and t.bg:
            label_wait = True
    return r(FAIL, f"{len(bad)} open(s) not chained with a wait") if bad else r(PASS)


def close_chained(run):
    """A close is not a call of its own when a cases command follows it."""
    tools = run.t.tools
    closes = [i for i, t in enumerate(tools) if "close" in t.subs]
    if not closes:
        return r(NA, "no close")
    bad = 0
    for i in closes:
        t = tools[i]
        later = any(u.segments and any(s in ("open", "wait", "note") for s in u.subs) for u in tools[i + 1:])
        if not later:
            continue
        if not any(s in ("open", "wait", "note") for s in t.subs):
            bad += 1
    return r(FAIL, f"{bad} close(s) run alone before more cases work") if bad else r(PASS)


def wait_output_read_once(run):
    """Each background wait's output is read once, unfiltered."""
    ids = [t.task_id for t in run.t.tools if "wait" in t.subs and t.task_id and t.task_status == "completed"]
    if not ids:
        return r(NA, "no finished background wait")
    bad = []
    for tid in ids:
        reads = [t for t in run.t.tools if tid + ".output" in json.dumps(t.input)]
        if len(reads) > 1:
            bad.append(f"{tid} read {len(reads)} times")
        for t in reads:
            if t.name == "Bash" and re.search(r"\|\s*(grep|jq|head|tail|sed|awk|python)", t.cmd):
                bad.append(f"{tid} filtered")
            if t.name == "Bash" and re.search(r"\b(tail|head|grep|jq)\b[^|]*" + tid, t.cmd):
                bad.append(f"{tid} read with a filter")
    return r(FAIL, "; ".join(bad)) if bad else r(PASS)


def _failed_writes(run):
    """Writes that exited 3 or 4, as (index of tool, case id, sub, args)."""
    out = []
    for t in run.t.tools:
        text = t.text()
        if not t.segments or (STALE not in text and REFUSED not in text):
            continue
        for s, a in t.segments:
            if s in WRITE_SUBS - {"open"}:
                out.append((t.index, first_positional(a), s, a))
    return out


def exit_3_4_handled(run):
    """After a write exits 3 or 4, the agent reads the case with show before
    writing to it again, and does not run the same write again."""
    failed = _failed_writes(run)
    if not failed:
        return r(NA, "no write exited 3 or 4")
    tools = run.t.tools
    for idx, cid, sub, args in failed:
        showed = False
        for t in tools[idx + 1:]:
            for s, a in t.segments:
                if s == "show" and cid and cid in a:
                    showed = True
                if s in WRITE_SUBS and cid and cid in a:
                    if not showed:
                        return r(FAIL, f"{s} on {cid} again without show")
                    if s == sub and a.strip() == args.strip():
                        return r(FAIL, f"ran the same {sub} on {cid} again")
        if not showed:
            return r(FAIL, f"never read {cid} after its {sub} failed")
    return r(PASS)


def withdrawn_respected(run):
    """Once the agent has seen a case withdrawn, it does not write to it or open it again."""
    withdrawn = [c for c in run.store if c["state"] == "withdrawn"
                 and any(e["event"] == "withdraw" and e["author"] == "human" for e in c["events"])]
    if not withdrawn:
        return r(NA, "nothing withdrawn by the human")
    for c in withdrawn:
        seen_at = None
        for t in run.t.tools:
            if seen_at is None:
                text = t.text()
                if c["id"] in text and "withdrawn" in text:
                    seen_at = t.index
                continue
            for s, a in t.segments:
                if s in WRITE_SUBS - {"open"} and c["id"] in a:
                    return r(FAIL, f"{s} on {c['id']} after seeing it withdrawn")
        later = [d for d in run.agent_cases if d["opened_at"] > c["updated_at"]
                 and difflib.SequenceMatcher(None, _norm(d["title"]), _norm(c["title"])).ratio() > 0.8]
        if later:
            return r(FAIL, f"opened {later[0]['id']} again after {c['id']} was withdrawn")
    return r(PASS)


def parked_handled(run):
    """After a park, the agent waits again with the parked line's next_since,
    so the resume wakes it."""
    parked = [c for c in run.store if any(e["event"] == "park" for e in c["events"])]
    if not parked:
        return r(NA, "nothing parked")
    resume = next((h for h in run.human if h["do"][0] == "resume"), None)
    if resume is not None and not resume.get("agent_waiting"):
        return r(FAIL, "no wait was running when the case was resumed")
    waits = [t for t in run.t.tools if "wait" in t.subs]
    for c in parked:
        saw = [t for t in waits if c["id"] in t.text() and '"parked"' in t.text()]
        if not saw:
            continue
        line = next((json.loads(x) for x in saw[0].text().splitlines()
                     if x.startswith("{") and c["id"] in x and '"parked"' in x), {})
        nxt = [t for t in waits if t.index > saw[0].index]
        if not nxt:
            return r(FAIL, "did not wait again after the park")
        since = flag_value(dict(nxt[0].segments)["wait"], "--since")
        # A variable holding the value is fine; a stale literal is not.
        if line.get("next_since") and since != line["next_since"] and not (since or "").startswith("$"):
            return r(FAIL, f"waited again with --since {since}, not {line['next_since']}")
    return r(PASS)


def message_handled(run):
    msgs = [c for c in run.store if c.get("kind") == "message"]
    if not msgs:
        return r(NA, "no message")
    bad = [c["id"] + " " + c["state"] for c in msgs
           if c["state"] != "withdrawn" and c["state"] != "closed"]
    return r(FAIL, "; ".join(bad)) if bad else r(PASS)


def status_beside_first_open(run):
    opens = [t for t in run.t.tools if "open" in t.subs]
    if not opens:
        return r(NA, "no open")
    first = opens[0]
    same = [t for t in run.t.tools if t.call_index == first.call_index and "status" in t.subs]
    if same:
        return r(PASS)
    before = [t for t in run.t.tools if t.index < first.index and "status" in t.subs]
    return r(FAIL, "status in a call of its own" if before else "no status with the first open")


def listed_on_resume(run):
    if not run.expect.get("resume"):
        return r(NA)
    writes = [t for t in run.t.tools if set(t.subs) & WRITE_SUBS]
    first = writes[0].index if writes else len(run.t.tools)
    listed = [t for t in run.t.tools if t.index < first and ("list" in t.subs or "show" in t.subs)]
    return r(PASS) if listed else r(FAIL, "wrote before reading its cases")


def noted_unclear(run):
    target = run.expect.get("note_on")
    if not target:
        return r(NA)
    cases = [c for c in run.store if re.search(target, c["title"], re.I)]
    for c in cases:
        if any(e["event"] == "note" and e["author"] == "agent" for e in c["events"]):
            return r(PASS)
    return r(FAIL, "did not note the unclear answer")


def outcome(run):
    """The task's own checks on what the agent did with the answers."""
    checks = run.expect.get("checks", [])
    if not checks:
        return r(NA)
    for chk in checks:
        kind = chk["type"]
        if kind == "file_contains":
            text = run.file(chk["path"]) or ""
            if not re.search(chk["pattern"], text, re.M):
                return r(FAIL, f"{chk['path']} lacks /{chk['pattern']}/")
        elif kind == "file_lacks":
            text = run.file(chk["path"]) or ""
            if re.search(chk["pattern"], text, re.M):
                return r(FAIL, f"{chk['path']} has /{chk['pattern']}/")
        elif kind == "file_exists":
            if run.file(chk["path"]) is None:
                return r(FAIL, f"no {chk['path']}")
        elif kind == "file_absent":
            if run.file(chk["path"]) is not None:
                return r(FAIL, f"{chk['path']} exists")
        elif kind == "case_state":
            cases = [c for c in run.store if re.search(chk["title_re"], c["title"], re.I)]
            if not cases or not any(c["state"] in chk["states"] for c in cases):
                return r(FAIL, f"case /{chk['title_re']}/ is {[c['state'] for c in cases]}")
        elif kind == "case_kind":
            cases = [c for c in run.counted_cases() if re.search(chk["title_re"], c["title"], re.I)]
            if not any(c["kind"] in chk["kinds"] for c in cases):
                return r(FAIL, f"no case of kind {chk['kinds']}: {[c['kind'] for c in cases]}")
        else:
            raise ValueError("unknown check " + kind)
    return r(PASS)


RULES = [
    ("no_human_writes", no_human_writes),
    ("no_forbidden", no_forbidden),
    ("wait_scoped", wait_scoped),
    ("revision_on_note_close", revision_on_note_close),
    ("closed_all_pickedup", closed_all_pickedup),
    ("no_duplicate", no_duplicate),
    ("opens_as_expected", opens_as_expected),
    ("no_secret", no_secret),
    ("wait_in_background", wait_in_background),
    ("wait_since", wait_since),
    ("one_pickup_wait_per_case", one_pickup_wait_per_case),
    ("open_chained", open_chained),
    ("close_chained", close_chained),
    ("wait_output_read_once", wait_output_read_once),
    ("exit_3_4_handled", exit_3_4_handled),
    ("withdrawn_respected", withdrawn_respected),
    ("parked_handled", parked_handled),
    ("message_handled", message_handled),
    ("status_beside_first_open", status_beside_first_open),
    ("listed_on_resume", listed_on_resume),
    ("noted_unclear", noted_unclear),
    ("outcome", outcome),
]


def cost(run):
    t = run.t
    decisions = sum(1 for c in run.store for e in c.get("events", [])
                    if e["author"] == "human" and e["event"] in ("answer", "withdraw", "park", "resume"))
    msgs = sum(1 for c in run.store if c.get("kind") == "message")
    decisions += msgs
    cls = t.call_classes()
    u = t.usage_totals()
    return {
        "api_calls": len(t.call_order),
        "turns": len(t.results),
        "tool_calls": len(t.tools),
        "cases_tool_calls": len(t.cases_tools()),
        "ask_calls": cls["ask"],
        "extra_calls": cls["extra"],
        "tokens": u,
        "context_tokens": u["input"] + u["cache_read"] + u["cache_write"],
        "cost_usd": round(t.cost_usd(), 4),
        "decisions": decisions,
        "skill_invoked": bool(t.skill_invoked),
        "skill_show_run": any(s == "skill" and "show" in a for _, s, a in run.segs()),
    }


def grade(run_dir, task):
    run = Run(run_dir, task)
    info = run.info
    infra = info.get("infra_error") or (run.t.errors and not run.t.tools and "api") or None
    out = {"task": task["id"], "arm": info["arm"], "run": os.path.basename(run_dir), "infra_error": infra,
           "timed_out": info.get("timed_out"), "rules": {}, "cost": None,
           # A human step the store refused, such as an --option on a case the
           # agent opened as a question: the agent's doing, reported apart.
           "human_step_failed": [e.get("stderr") for e in info.get("human_errors") or []]}
    if infra and not run.t.tools:
        return out
    for name, fn in RULES:
        try:
            out["rules"][name] = fn(run)
        except Exception as e:  # a grader bug is reported, not scored
            out["rules"][name] = {"result": "error", "why": f"{type(e).__name__}: {e}"}
    out["cost"] = cost(run)
    return out
