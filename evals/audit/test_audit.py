#!/usr/bin/env python3
"""Tests of the offline audit on hand-built session files. No model, no real transcripts.
`python3 evals/audit/test_audit.py` runs them."""
import json
import os
import shutil
import tempfile
import unittest

import audit
import history
import rules_cases
import rules_team_lead as tl
import transcript


class SessionFile:
    """Writes a Claude Code session transcript, in the real format."""

    def __init__(self, start="2026-09-29T10:00:00Z"):
        self.lines = []
        self.n = 0
        self.start = start

    def user(self, text):
        self.lines.append({"type": "user", "timestamp": self.start, "message": {"role": "user", "content": text}})

    def tool(self, name, inp, result="ok", sidechain=False, same_call=False):
        self.n += 1
        tid = f"t{self.n}"
        mid = f"m{self.n - 1}" if same_call else f"m{self.n}"
        self.lines.append({"type": "assistant", "isSidechain": sidechain, "timestamp": self.start,
                           "message": {"id": mid, "usage": {"input_tokens": 10},
                                       "content": [{"type": "tool_use", "id": tid, "name": name, "input": inp}]}})
        self.lines.append({"type": "user", "isSidechain": sidechain, "timestamp": self.start,
                           "message": {"content": [{"type": "tool_result", "tool_use_id": tid, "content": result}]}})
        return tid

    def bash(self, cmd, bg=False, **kw):
        inp = {"command": cmd}
        if bg:
            inp["run_in_background"] = True
            kw.setdefault("result", f"Command running in background with ID: b{self.n + 1}. "
                                    f"Output is being written to: /nonexistent/b{self.n + 1}.output")
        return self.tool("Bash", inp, **kw)

    def notify(self, task_id, status="completed"):
        self.lines.append({"type": "queue-operation", "operation": "enqueue",
                           "content": f"<task-notification>\n<task-id>{task_id}</task-id>\n<status>{status}</status>"
                                      "\n</task-notification>"})

    def write(self, root, name="s1"):
        d = os.path.join(root, "-Users-x-proj")
        os.makedirs(d, exist_ok=True)
        path = os.path.join(d, name + ".jsonl")
        with open(path, "w") as f:
            for line in self.lines:
                f.write(json.dumps(line) + "\n")
        return path


class Audit(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp()

    def tearDown(self):
        shutil.rmtree(self.root)

    def session(self, f):
        return history.Session(f.write(self.root))

    def test_notifications_end_background_tasks(self):
        f = SessionFile()
        f.user("go")
        f.bash('cases wait --label L --worker w --pickup --by w --since x', bg=True)
        f.bash("echo other")
        f.notify("b1")
        f.bash("echo after")
        s = self.session(f)
        wait = s.t.tools[0]
        self.assertEqual((wait.task_status, wait.done_at), ("completed", 2))

    def test_sidechains_heredocs_and_quotes_are_not_commands(self):
        f = SessionFile()
        f.bash("cases answer X --option 1", sidechain=True)
        f.bash("python3 - <<'EOF'\ns = 'cases close X --outcome y'\nEOF")
        f.bash('grep -n "cases park" SKILL.md')
        f.bash('id=$(cases open --kind fyi --title "a; b" --worker w) && cases wait --id "$id" --since "$id" --pickup',
               bg=True)
        s = self.session(f)
        self.assertEqual([t.subs for t in s.t.tools], [[], [], ["open", "wait"]])

    def test_cases_calls_on_another_store_are_not_use(self):
        f = SessionFile()
        f.bash("cases open --kind fyi --title t --store /tmp/s.db")
        f.bash("CASES_STORE=/tmp/s.db cases wait --id X --pickup")
        self.assertFalse(rules_cases.select(self.session(f)))

    def test_turns_start_at_typed_messages(self):
        f = SessionFile()
        f.user("first")
        f.bash("gh pr checks 3")
        f.lines.append({"type": "user", "message": {"content": "<task-notification>\n<task-id>x</task-id>"}})
        f.bash("gh pr merge 3 --squash")
        f.user("second")
        f.bash("gh pr merge 4 --squash")
        s = self.session(f)
        self.assertEqual(s.turn_of, [1, 1, 2])
        self.assertEqual(tl.merge_after_checks(s), (1, 1))

    def test_team_lead_messages(self):
        block = "Reporting. I am lead, the only session that reads your output. Final report fields: x."
        f = SessionFile()
        f.bash("herdr agent start bun-pins --kind claude --pane p -- --name bun-pins --model opus --effort medium")
        f.tool("SendMessage", {"to": "bun-pins", "message": "Do it.\n\n" + block, "notify_when_idle": True})
        f.tool("SendMessage", {"to": "bun-pins", "message": "Then run `/code-review --fix`."})
        f.tool("SendMessage", {"to": "wide-layout", "message": block + "\n\nPS: more"})
        f.tool("SendMessage", {"to": "main", "message": "to the parent"})
        f.tool("SendMessage", {"to": "wide-layout", "message": "say what /code-review found; "
                               "run `/code-review high --fix`; `/code-review --fix` at medium"})
        s = self.session(f)
        self.assertTrue(tl.select(s))
        self.assertEqual(tl.notify_when_idle(s), (1, 3))
        self.assertEqual(tl.brief_ends_with_reporting(s), (1, 1))
        self.assertEqual(tl.code_review_level(s), (2, 1))
        self.assertEqual(tl.agent_start_model_effort(s), (1, 0))
        self.assertEqual(tl.worker_names(s), (1, 0))

    def test_effort_is_raised_on_built(self):
        f = SessionFile()
        f.bash("herdr agent start w --kind claude -- --name w --model opus --effort medium")
        f.user('<cross-session-message from="uds:x" from-name="w">built</cross-session-message>')
        f.bash('herdr agent prompt w "/effort high"')
        f.tool("SendMessage", {"to": "w", "message": "verify", "notify_when_idle": True})
        f.user('<cross-session-message from="uds:y" from-name="v">built</cross-session-message>')
        f.tool("SendMessage", {"to": "v", "message": "verify", "notify_when_idle": True})
        self.assertEqual(tl.effort_raised_on_built(self.session(f)), (1, 1))

    def test_two_label_waits_at_once_fail(self):
        f = SessionFile()
        f.bash("herdr agent start w -- --model opus --effort medium")
        f.bash("cases wait --label L --worker lead --pickup --by lead --since a", bg=True)
        f.bash("cases wait --label L --worker lead --pickup --by lead --since b", bg=True)
        f.notify("b2")
        f.notify("b3")
        f.bash("cases wait --label L --worker lead --pickup --by lead --since c", bg=True)
        self.assertEqual(tl.one_label_wait(self.session(f)), (2, 1))

    def test_a_stopped_wait_has_ended(self):
        f = SessionFile()
        f.bash("herdr agent start w -- --model opus --effort medium")
        f.bash("cases wait --label L --worker lead --pickup --by lead --since a", bg=True)
        f.tool("TaskStop", {"task_id": "b2"})
        f.bash("cases wait --label L --worker lead --pickup --by lead --since b", bg=True)
        s = self.session(f)
        self.assertEqual((s.t.tools[1].task_status, s.t.tools[1].done_at), ("stopped", 2))
        self.assertEqual(tl.one_label_wait(s), (2, 0))

    def test_a_wait_notified_before_its_stop_completed(self):
        f = SessionFile()
        f.bash("cases wait --label L --worker lead --pickup --by lead --since a", bg=True)
        f.notify("b1")
        f.tool("TaskStop", {"task_id": "b1"})
        s = self.session(f)
        self.assertEqual((s.t.tools[0].task_status, s.t.tools[0].done_at), ("completed", 1))

    def test_waits_on_two_cases_named_by_one_variable_are_two(self):
        f = SessionFile()
        f.user("go")
        f.bash('id=$(cases open --kind fyi --title a --worker w) && cases wait --id "$id" --since "$id" --pickup',
               bg=True)
        f.bash('id=$(cases open --kind fyi --title b --worker w) && cases wait --id "$id" --since "$id" --pickup',
               bg=True)
        s = self.session(f)
        rules_cases.select(s)
        self.assertEqual(dict((n, c) for n, _, c in rules_cases.RULES)["one_pickup_wait_per_case"](s), (1, 0))

    def test_two_starts_on_one_line_are_two(self):
        f = SessionFile()
        f.bash("herdr agent start a -- --model opus --effort medium && herdr agent start b1 --kind claude")
        s = self.session(f)
        self.assertEqual(tl.one_start_per_call(s), (0, 1))
        self.assertEqual(tl.worker_names(s), (1, 1))
        self.assertEqual(tl.agent_start_model_effort(s), (1, 1))

    def test_a_skill_mid_turn_does_not_start_a_turn(self):
        f = SessionFile()
        f.user("merge it")
        f.bash("gh pr checks 3")
        f.tool("Skill", {"skill": "code-review", "args": "--fix high"})
        f.lines.append({"type": "user", "isMeta": True,
                        "message": {"content": [{"type": "text", "text": "Base directory for this skill: x"}]}})
        f.bash("gh pr merge 3 --squash")
        s = self.session(f)
        self.assertEqual(s.turn_of, [1, 1, 1])
        self.assertEqual(tl.merge_after_checks(s), (1, 0))
        self.assertEqual(tl.code_review_level(s), (1, 0))

    def cost_of(self, *cmds):
        f = SessionFile()
        f.user("go")
        for cmd in cmds:
            f.bash(cmd)
        return rules_cases.cost(self.session(f))

    def test_work_chained_with_bookkeeping_is_not_a_cases_only_call(self):
        c = self.cost_of('git commit -qam "Pin bun" && cases close A --revision 3 --outcome "Pinned bun."',
                         "make test; cases close A --revision 3 --outcome done",
                         "cargo build && id=$(cases open --kind fyi --title t --worker w) && cases wait --id \"$id\"")
        self.assertEqual((c["ask"], c["extra"]), (0, 0))

    def test_bookkeeping_and_its_glue_are_cases_only_calls(self):
        c = self.cost_of(
            'cases close A --revision 3 --outcome "Ran make; all green" && '
            'id=$(cases open --kind fyi --title t --worker w) && cases wait --id "$id" --since "$id" --pickup --by w',
            "cat > q.md <<'EOF'\nWe could run make deploy && rm -rf build; which?\nEOF\n"
            "cases open --kind question --title q --body-file q.md --worker w",
            'cd /repo && cases show A --answer | jq .answer',
            'echo "$id"; cases list --worker w',
            "cases wait --label L --worker w --pickup --by w --since x || true")
        self.assertEqual((c["ask"], c["extra"]), (2, 3))

    def test_only_commands_that_do_no_work_are_bookkeeping(self):
        work = [
            "if ! git diff --quiet; then git commit -am wip; fi; cases close A --outcome done",
            "true && { git push; } && cases close A",
            "cases close A --outcome-file - <<EOF && git push\nDone.\nEOF",
            "echo $(git push origin main) && cases close A",
            'echo "$(make deploy)"; cases close A',
            "id=$(cases list; git push)",
            "X=$(cases list) git push",
            'cases close A --outcome "$(git log -1 --format=%h)"',
            "( cd x && make ) ; cases close A --outcome done",
            "cat src/main.rs; cases list",
            "cat > src/main.rs <<'EOF'\nfn main(){}\nEOF\ncases close A",
            "echo hi > src/x.rs; cases list",
            "cases wait --id A & git push",
            "grep -rn foo src/ ; cases list",
            "tail -20 /tmp/x/b3.output",
        ]
        bookkeeping = [
            'id="$(cases open --kind fyi --title "a)b; c" --worker w)" && cases wait --id "$id"',
            "FOO=1 id=$(cases open --title t)",
            "cat > q.md <<'EOF'\nWe could run make deploy && rm -rf build\nEOF\ncases open --title q --body-file q.md",
            "for i in A B; do cases show $i --answer; done",
            "if cases close A; then echo ok; fi",
            "# close it\ncases close A --outcome done",
            "cases list --json | grep -c open",
            "cases show A --answer | jq -r .answer 2>&1",
            "cases list > /dev/null 2>&1 && echo ok",
            "cat /tmp/x/b7.output",
        ]
        for cmd in work:
            self.assertFalse(transcript.bookkeeping_only(cmd, {"b7"}), cmd)
        for cmd in bookkeeping:
            self.assertTrue(transcript.bookkeeping_only(cmd, {"b7"}), cmd)

    def test_reading_a_waits_output_is_a_cases_only_call(self):
        f = SessionFile()
        f.user("go")
        f.bash("cases wait --id A --since A --pickup --by w", bg=True)
        f.tool("Read", {"file_path": "/nonexistent/b1.output"})
        f.bash("cat /nonexistent/b1.output && git push")
        c = rules_cases.cost(self.session(f))
        self.assertEqual((c["ask"], c["extra"]), (0, 2))

    def test_reading_another_background_tasks_output_is_not_a_wait_read(self):
        f = SessionFile()
        f.user("go")
        f.bash("cargo test", bg=True)
        f.bash("tail -20 /nonexistent/b1.output")
        c = rules_cases.cost(self.session(f))
        self.assertEqual((c["ask"], c["extra"]), (0, 0))

    def test_the_report_holds_no_transcript_text(self):
        f = SessionFile()
        f.user("the secret word is pelican")
        f.bash('cases open --kind decision --title "pelican plans" --worker w --option A --option B')
        f.bash("cases wait --id X --since X --pickup --by w")
        f.write(self.root)
        out = os.path.join(self.root, "report.md")
        audit.main(["--rules", rules_cases.__file__, "--rules", tl.__file__,
                    "--glob", os.path.join(self.root, "*", "*.jsonl"), "--out", out])
        with open(out) as fh:
            report = fh.read()
        self.assertNotIn("pelican", report)
        self.assertIn("| wait_in_background | n/a | 0% ", report)
        self.assertIn("s1 (after #78)", report)


if __name__ == "__main__":
    unittest.main()
