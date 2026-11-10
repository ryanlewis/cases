#!/usr/bin/env python3
"""Tests of the graders on hand-built runs: a good run and bad runs that each
break one rule. `python3 evals/cases-skill/test_graders.py` runs them; no model
is called."""
import json
import os
import shutil
import tempfile
import unittest

import graders


class RunBuilder:
    """Writes a run directory: a stream-json transcript, a store and a human log."""

    def __init__(self):
        self.events = []
        self.n = 0
        self.outputs = {}
        self.store = []
        self.human = []
        self.info = {"task": "t", "arm": "with", "infra_error": None, "timed_out": False, "seeded": []}

    def _id(self):
        self.n += 1
        return f"t{self.n}"

    def bash(self, cmd, out="", exit=0, bg=False, bg_out=None, same_call=False):
        tid = self._id()
        mid = f"m{self.n}" if not same_call else f"m{self.n - 1}"
        inp = {"command": cmd}
        if bg:
            inp["run_in_background"] = True
        self.events.append({"type": "assistant", "message": {"id": mid, "usage": {"input_tokens": 1}, "content": [
            {"type": "tool_use", "id": tid, "name": "Bash", "input": inp}]}})
        if bg:
            task = "bg" + tid
            res = f"Command running in background with ID: {task}. Output is being written to: /x/{task}.output"
            self.events.append({"type": "user", "message": {"content": [
                {"type": "tool_result", "tool_use_id": tid, "content": res}]}})
            self.events.append({"type": "system", "subtype": "task_notification", "task_id": task,
                                "tool_use_id": tid, "status": "completed", "output_file": f"/x/{task}.output"})
            self.outputs[task] = bg_out or ""
            return task
        content = out if exit == 0 else f"Exit code {exit}\n{out}"
        self.events.append({"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": tid, "content": content, "is_error": exit != 0}]}})
        return tid

    def command(self, contains, new):
        """Replaces the last Bash command that contains `contains`."""
        for e in reversed(self.events):
            for c in e.get("message", {}).get("content", []) if e["type"] == "assistant" else []:
                if c.get("name") == "Bash" and contains in c["input"]["command"]:
                    c["input"]["command"] = new
                    return
        raise KeyError(contains)

    def read(self, path):
        tid = self._id()
        self.events.append({"type": "assistant", "message": {"id": f"m{self.n}", "usage": {}, "content": [
            {"type": "tool_use", "id": tid, "name": "Read", "input": {"file_path": path}}]}})
        self.events.append({"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": tid, "content": "..."}]}})

    def case(self, cid, state, kind="decision", title="Pin or float?", events=("open",), worker="w", **kw):
        evs = []
        for e in events:
            author = "human" if e in ("answer", "park", "resume") or e.startswith("h-") else "agent"
            evs.append({"event": e.replace("h-", ""), "author": author, "data": {}})
        c = {"id": cid, "state": state, "kind": kind, "title": title, "worker": worker,
             "actor": {"kind": "agent"}, "opened_at": f"2026-09-29T10:00:0{len(self.store)}Z",
             "updated_at": "2026-09-29T10:10:00Z", "events": evs, "labels": kw.get("labels", []),
             "revision": len(evs)}
        c.update({k: v for k, v in kw.items() if k != "labels"})
        self.store.append(c)

    def write(self, root):
        d = tempfile.mkdtemp(dir=root)
        with open(os.path.join(d, "transcript.jsonl"), "w") as f:
            for e in self.events + [{"type": "result", "total_cost_usd": 0.01}]:
                f.write(json.dumps(e) + "\n")
        os.makedirs(os.path.join(d, "tasks"))
        for tid, text in self.outputs.items():
            with open(os.path.join(d, "tasks", tid + ".output"), "w") as f:
                f.write(text)
        with open(os.path.join(d, "store.json"), "w") as f:
            json.dump(self.store, f)
        with open(os.path.join(d, "human.jsonl"), "w") as f:
            for h in self.human:
                f.write(json.dumps(h) + "\n")
        with open(os.path.join(d, "run.json"), "w") as f:
            json.dump(self.info, f)
        return d


LINE = '{"id":"UkLWZ","state":"pickedup","kind":"decision","revision":3,"next_since":"2026-09-29T10:05:00Z"}'
TASK = {"id": "t", "expect": {"opens": 1}}


def good():
    """open && wait in the background, status beside it, one cat, then work && close."""
    b = RunBuilder()
    task = b.bash('id=$(cases open --kind decision --urgency today --worker w --title "Pin or float?" '
                  '--option A --option B) && cases wait --id "$id" --since "$id" --pickup --by w',
                  bg=True, bg_out=LINE)
    b.bash("cases status", out="not running", exit=1, same_call=True)
    b.bash(f"cat /x/{task}.output", out=LINE)
    b.bash('sed -i "" s/a/b/ f && cases close UkLWZ --revision 3 --outcome "Pinned it."', out="UkLWZ closed")
    b.case("UkLWZ", "closed", events=("open", "answer", "pickup", "close"))
    return b


class Graders(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp()

    def tearDown(self):
        shutil.rmtree(self.root)

    def grade(self, b, task=TASK):
        return {k: v["result"] for k, v in graders.grade(b.write(self.root), task)["rules"].items()}

    def test_a_good_run_passes_every_rule_that_applies(self):
        g = self.grade(good())
        self.assertEqual({k for k, v in g.items() if v == "fail"}, set(), g)
        for rule in ("wait_scoped", "revision_on_note_close", "closed_all_pickedup", "wait_in_background",
                     "wait_since", "open_chained", "close_chained", "wait_output_read_once",
                     "status_beside_first_open", "opens_as_expected"):
            self.assertEqual(g[rule], "pass", rule)

    def test_cost_counts_only_calls_that_are_all_bookkeeping(self):
        b = good()
        b.bash("cat > q.md <<'EOF'\ncases open --kind fyi, as before\nEOF\n"
               "cases close UkLWZ --revision 3 --outcome-file q.md", out="UkLWZ closed")
        c = graders.grade(b.write(self.root), TASK)["cost"]
        # The open with its wait is the ask; reading the wait's output and the close
        # whose body file mentions an open are extra; the sed with its close is work.
        self.assertEqual((c["ask_calls"], c["extra_calls"]), (1, 2))

    def test_a_close_without_revision_fails(self):
        b = good()
        b.command("cases close", 'cases close UkLWZ --outcome "x"')
        self.assertEqual(self.grade(b)["revision_on_note_close"], "fail")

    def test_help_is_not_a_use(self):
        b = good()
        b.bash("cases close --help; cases wait --help")
        g = self.grade(b)
        self.assertEqual(g["revision_on_note_close"], "pass")
        self.assertEqual(g["wait_in_background"], "pass")

    def test_an_unscoped_pickup_wait_fails(self):
        b = good()
        b.bash("cases wait --label L --pickup --by w --since x", bg=True)
        self.assertEqual(self.grade(b)["wait_scoped"], "fail")
        # The listening wait for messages is scoped by the worker name.
        c = good()
        c.bash("cases wait --worker w --pickup --by w", bg=True)
        self.assertEqual(self.grade(c)["wait_scoped"], "pass")

    def test_a_foreground_wait_fails(self):
        b = good()
        b.bash("cases wait --id UkLWZ --since UkLWZ --pickup --by w")
        self.assertEqual(self.grade(b)["wait_in_background"], "fail")

    def test_the_human_commands_fail(self):
        for cmd in ("cases answer UkLWZ --option 1", "cases park UkLWZ", "cases message --to w --body x --as me",
                    "cases withdraw UkLWZ --as me --reason x"):
            b = good()
            b.bash(cmd)
            self.assertEqual(self.grade(b)["no_human_writes"], "fail", cmd)

    def test_sweep_and_sqlite_fail(self):
        for cmd in ("cases sweep --worker w", "sqlite3 ~/.local/share/cases/cases.db .tables"):
            b = good()
            b.bash(cmd)
            self.assertEqual(self.grade(b)["no_forbidden"], "fail", cmd)

    def test_reading_the_output_twice_or_filtered_fails(self):
        b = good()
        b.read("/x/bgt1.output")
        self.assertEqual(self.grade(b)["wait_output_read_once"], "fail")
        b = good()
        b.command("cat /x/bgt1.output", "cat /x/bgt1.output | jq .")
        self.assertEqual(self.grade(b)["wait_output_read_once"], "fail")

    def test_a_close_alone_before_the_next_open_fails(self):
        b = good()
        b.bash('cases open --kind fyi --urgency whenever --title "Done" --worker w && '
               'cases wait --id X --since X --pickup --by w', bg=True)
        self.assertEqual(self.grade(b)["close_chained"], "fail")
        b.case("X", "open", kind="fyi", title="Done")
        # The same, but the close and the next open in one command, passes.
        c = good()
        c.command("cases close", (
            'cases close UkLWZ --revision 3 --outcome "x" && id=$(cases open --kind fyi --title Done '
            '--worker w) && cases wait --id "$id" --since "$id" --pickup --by w'))
        self.assertEqual(self.grade(c)["close_chained"], "pass")

    def test_an_open_without_its_wait_fails(self):
        b = RunBuilder()
        b.bash('cases open --kind decision --title "Pin?" --worker w --option A --option B', out="UkLWZ")
        b.bash("cases wait --id UkLWZ --since UkLWZ --pickup --by w", bg=True, bg_out=LINE)
        b.case("UkLWZ", "pickedup", events=("open", "answer", "pickup"))
        g = self.grade(b)
        self.assertEqual(g["open_chained"], "fail")
        self.assertEqual(g["closed_all_pickedup"], "fail")

    def test_a_duplicate_open_fails(self):
        b = good()
        b.case("Ab123", "open", title="Pin or float?")
        g = self.grade(b)
        self.assertEqual(g["no_duplicate"], "fail")
        self.assertEqual(g["opens_as_expected"], "fail")

    def test_a_secret_in_the_store_fails(self):
        b = good()
        b.store[0]["body"] = "token is sk_live_ABC123"
        task = {"id": "t", "expect": {"opens": 1, "secret": "sk_live_ABC123"}}
        self.assertEqual(self.grade(b, task)["no_secret"], "fail")
        b.store[0]["body"] = "the token in .env"
        self.assertEqual(self.grade(b, task)["no_secret"], "pass")

    def test_exit_4_then_show_passes_and_a_blind_retry_fails(self):
        stale = "Error: the case changed since it was read: read at revision 1, now at 2"
        b = RunBuilder()
        b.bash("cases note UkLWZ --revision 1 --body-file v.md", out=stale, exit=4)
        b.bash("cases show UkLWZ --answer", out='{"state":"answered"}')
        b.bash('cases pickup UkLWZ --revision 2 --by w && cases close UkLWZ --revision 3 --outcome "x"')
        b.case("UkLWZ", "closed", events=("open", "answer", "pickup", "close"))
        self.assertEqual(self.grade(b, {"id": "t", "expect": {}})["exit_3_4_handled"], "pass")
        c = RunBuilder()
        c.bash("cases note UkLWZ --revision 1 --body-file v.md", out=stale, exit=4)
        c.bash("cases note UkLWZ --revision 2 --body-file v.md")
        c.case("UkLWZ", "open", events=("open", "answer", "note"))
        self.assertEqual(self.grade(c, {"id": "t", "expect": {}})["exit_3_4_handled"], "fail")
        # A show, then the same write again in a later call, still fails.
        d = RunBuilder()
        d.bash("cases note UkLWZ --revision 1 --body-file v.md", out=stale, exit=4)
        d.bash("cases show UkLWZ --answer", out='{"state":"answered"}')
        d.bash("cases note UkLWZ --revision 1 --body-file v.md", out=stale, exit=4)
        d.case("UkLWZ", "open", events=("open", "answer"))
        self.assertEqual(self.grade(d, {"id": "t", "expect": {}})["exit_3_4_handled"], "fail")

    def test_a_separator_inside_quotes_does_not_cut_the_arguments(self):
        b = good()
        b.command("cases close", 'cases close UkLWZ --outcome "ran vacuum; held reindex | later" --revision 3')
        self.assertEqual(self.grade(b)["revision_on_note_close"], "pass")

    def test_writing_to_a_case_seen_withdrawn_fails(self):
        wd = '{"id":"UkLWZ","state":"withdrawn","kind":"decision","revision":4}'
        b = RunBuilder()
        b.bash('cases close UkLWZ --revision 3 --outcome "x"', out="Error: a case that is withdrawn does not take close", exit=3)
        b.bash("cases show UkLWZ --answer", out=wd)
        b.case("UkLWZ", "withdrawn", events=("open", "answer", "pickup", "h-withdraw"))
        self.assertEqual(self.grade(b, {"id": "t", "expect": {}})["withdrawn_respected"], "pass")
        b.bash('cases close UkLWZ --revision 4 --outcome "x"', out="Error: does not take close", exit=3)
        self.assertEqual(self.grade(b, {"id": "t", "expect": {}})["withdrawn_respected"], "fail")

    def test_waiting_again_after_a_park_with_the_old_since_fails(self):
        park = '{"id":"UkLWZ","state":"parked","kind":"stuck","revision":2,"next_since":"2026-09-29T10:05:00Z"}'
        b = RunBuilder()
        b.bash('id=$(cases open --kind stuck --title S --worker w) && cases wait --id "$id" --since "$id" --pickup --by w',
               bg=True, bg_out=park)
        b.bash("cases wait --id UkLWZ --since UkLWZ --pickup --by w", bg=True)
        b.case("UkLWZ", "closed", kind="stuck", title="S", events=("open", "park", "resume", "answer", "pickup", "close"))
        b.human = [{"do": ["park", "UkLWZ"], "agent_waiting": True}, {"do": ["resume", "UkLWZ"], "agent_waiting": True}]
        self.assertEqual(self.grade(b)["parked_handled"], "fail")
        b.command("--since UkLWZ", "cases wait --id UkLWZ --since 2026-09-29T10:05:00Z --pickup --by w")
        self.assertEqual(self.grade(b)["parked_handled"], "pass")

    def test_two_pickup_waits_on_one_label_at_once_fail(self):
        b = RunBuilder()
        b.bash("cases wait --label L --worker w --pickup --by w --since x", bg=True)
        # The first wait has not ended when the second starts.
        b.events.pop()
        b.bash("cases wait --label L --worker w --pickup --by w --since x", bg=True)
        self.assertEqual(self.grade(b, {"id": "t", "expect": {}})["one_pickup_wait_per_case"], "fail")

    def test_a_wait_the_agent_stopped_has_ended(self):
        b = RunBuilder()
        b.bash("cases wait --label L --worker w --pickup --by w --since x", bg=True)
        b.events.pop()  # no notification: the agent stops it instead
        tid = b._id()
        b.events.append({"type": "assistant", "message": {"id": "stop", "usage": {}, "content": [
            {"type": "tool_use", "id": tid, "name": "TaskStop", "input": {"task_id": "bgt1"}}]}})
        b.bash("cases wait --label L --worker w --pickup --by w --since y", bg=True)
        run = graders.Run(b.write(self.root), {"id": "t", "expect": {}})
        first = run.t.tools[0]
        self.assertEqual((first.task_status, first.done_at), ("stopped", 1))
        self.assertEqual(graders.one_pickup_wait_per_case(run)["result"], "pass")

    def test_a_notification_before_the_stop_wins(self):
        b = RunBuilder()
        b.bash("cases wait --id X --since X --pickup --by w", bg=True, bg_out="line")
        tid = b._id()
        b.events.append({"type": "assistant", "message": {"id": "stop", "usage": {}, "content": [
            {"type": "tool_use", "id": tid, "name": "TaskStop", "input": {"task_id": "bgt1"}}]}})
        run = graders.Run(b.write(self.root), {"id": "t", "expect": {}})
        # It completed (notified at 1) before the stop (at 1 too, the call after it): completed stands.
        self.assertEqual((run.t.tools[0].task_status, run.t.tools[0].done_at), ("completed", 1))

    def test_a_stop_before_the_notification_wins(self):
        b = RunBuilder()
        b.bash("cases wait --id X --since X --pickup --by w", bg=True)
        note = b.events.pop()
        tid = b._id()
        b.events.append({"type": "assistant", "message": {"id": "stop", "usage": {}, "content": [
            {"type": "tool_use", "id": tid, "name": "TaskStop", "input": {"task_id": "bgt1"}}]}})
        b.events.append(dict(note, status="killed"))
        run = graders.Run(b.write(self.root), {"id": "t", "expect": {}})
        self.assertEqual((run.t.tools[0].task_status, run.t.tools[0].done_at), ("stopped", 1))

    def test_a_failed_stop_stopped_nothing(self):
        b = RunBuilder()
        b.bash("cases wait --label L --worker w --pickup --by w --since x", bg=True)
        b.events.pop()
        tid = b._id()
        b.events.append({"type": "assistant", "message": {"id": "stop", "usage": {}, "content": [
            {"type": "tool_use", "id": tid, "name": "TaskStop", "input": {"task_id": "bgt1"}}]}})
        b.events.append({"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": tid, "content": "no such task", "is_error": True}]}})
        run = graders.Run(b.write(self.root), {"id": "t", "expect": {}})
        self.assertEqual((run.t.tools[0].task_status, run.t.tools[0].done_at), (None, None))

    def test_opening_a_case_when_none_was_needed_fails(self):
        b = good()
        self.assertEqual(self.grade(b, {"id": "t", "expect": {"opens": 0}})["opens_as_expected"], "fail")


if __name__ == "__main__":
    unittest.main()
