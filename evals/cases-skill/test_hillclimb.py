#!/usr/bin/env python3
"""Tests of the hillclimb driver with a fake `claude` and a fake `cases`: no
model is called. `python3 evals/cases-skill/test_hillclimb.py` runs them.

The fake `claude` is both the proposer and the agent. As the proposer it
returns a diff that rewrites a marker line in a scratch repo's SKILL.md. As
the agent it reads that marker from the plugin it was given and prints a
canned transcript: a background wait, a close with or without --revision
(by a pass pattern over each task's runs), an `answer` when told to break
no_human_writes, and as many API calls as the marker says. So each round's
grades, and so each keep/revert branch, follow from the marker.
"""
import argparse
import json
import os
import shutil
import subprocess
import tempfile
import unittest

import hillclimb

FAKE_CLAUDE = r'''#!/usr/bin/env python3
import difflib, fcntl, hashlib, json, os, re, sys
argv = sys.argv[1:]
if argv[:2] == ["auth", "status"]:
    print(json.dumps({"loggedIn": True}))
    sys.exit(0)
# The runner gives a session a bare environment, so the settings live beside the fake.
env = json.load(open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "fake-env.json")))
repo = env["FAKE_REPO"]
if "--input-format" not in argv:
    # The proposer: one diff that sets the marker line to FAKE_NEXT.
    prompt = sys.stdin.read()
    open(os.path.join(env["FAKE_STATE"], "proposer-input.txt"), "w").write(prompt)
    rel = "crates/cli/skill/SKILL.md"
    old = open(os.path.join(repo, rel)).read()
    new = re.sub(r"^<!-- fake: .* -->$", "<!-- fake: " + env["FAKE_NEXT"] + " -->", old, flags=re.M)
    diff = "".join(difflib.unified_diff(old.splitlines(True), new.splitlines(True), "a/" + rel, "b/" + rel))
    reply = {"hypothesis": "set the marker to " + env["FAKE_NEXT"], "diff": diff}
    print(json.dumps({"type": "result", "result": json.dumps(reply)}))
    sys.exit(0)
# The agent.
prompt = json.loads(sys.stdin.readline())["message"]["content"]
plugin = argv[argv.index("--plugin-dir") + 1]
marker = re.search(r"<!-- fake: (.*) -->", open(os.path.join(plugin, "skills", "cases", "SKILL.md")).read()).group(1)
conf = dict(kv.split("=") for kv in marker.split())
pattern = conf["held"] if "HELDOUT" in prompt else conf["train"]
key = hashlib.sha1((prompt + marker).encode()).hexdigest()
path = os.path.join(env["FAKE_STATE"], key)
with open(path + ".lock", "w") as lock:
    fcntl.flock(lock, fcntl.LOCK_EX)
    n = int(open(path).read()) if os.path.exists(path) else 0
    open(path, "w").write(str(n + 1))
good = pattern[n % len(pattern)] == "1"
def emit(e):
    print(json.dumps(e), flush=True)
emit({"type": "system", "subtype": "init"})
calls = int(conf.get("calls", 1))
for i in range(calls - 1):
    emit({"type": "assistant", "message": {"id": "m%d" % i, "usage": {"input_tokens": 1000}, "content": [{"type": "text", "text": "."}]}})
tools = [("t1", "cases wait --id UkLWZ --since UkLWZ --pickup --by w", True),
         ("t2", "cases close UkLWZ " + ("--revision 3 " if good else "") + '--outcome "done"', False)]
if conf.get("human") == "1":
    tools.append(("t3", "cases answer UkLWZ --option 1", False))
content = [{"type": "tool_use", "id": t, "name": "Bash", "input": dict({"command": c}, **({"run_in_background": True} if bg else {}))} for t, c, bg in tools]
emit({"type": "assistant", "message": {"id": "last", "usage": {"input_tokens": 1000}, "content": content}})
for t, c, bg in tools:
    text = "Command running in background with ID: b%s. Output is being written to: /nonexistent/b%s.output" % (t, t) if bg else "ok"
    emit({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": t, "content": text}]}})
emit({"type": "system", "subtype": "task_notification", "task_id": "bt1", "tool_use_id": "t1", "status": "completed"})
emit({"type": "rate_limit_event", "rate_limit_info": {"status": "allowed", "utilization": float(env.get("FAKE_UTIL", "0.1"))}})
emit({"type": "result", "subtype": "success", "total_cost_usd": 0.01 * calls})
sys.stdin.read()
'''

TASK = """
situation = "{situation}"
source = "test"
worker = "{worker}"
max_minutes = 1
prompt = "{prompt}"

[expect]
opens = 0
"""


def fake_build(repo):
    """A `build` that writes a fake `cases` with the skill embedded, as the real build embeds it."""
    def build(binary_out):
        with open(os.path.join(repo, "crates/cli/skill/SKILL.md")) as f:
            body = f.read()
        with open(binary_out, "w") as f:
            f.write("#!/bin/sh\ncase \"$1\" in\n"
                    "skill) cat <<'SKILL'\n---\nname: cases\ndescription: test\n---\n\n" + body + "SKILL\n;;\n"
                    "list) echo '{\"list\": []}' ;;\n"
                    "*) echo 'cases 0.0.0-fake' ;;\nesac\n")
        os.chmod(binary_out, 0o755)
    return build


class Hillclimb(unittest.TestCase):
    BASE = "train=000 held=110 calls=4"

    @classmethod
    def setUpClass(cls):
        cls.root = tempfile.mkdtemp()
        cls.state = os.path.join(cls.root, "state")
        os.makedirs(cls.state)
        cls.claude = os.path.join(cls.root, "claude")
        with open(cls.claude, "w") as f:
            f.write(FAKE_CLAUDE)
        os.chmod(cls.claude, 0o755)
        cls.tasks_root = os.path.join(cls.root, "suite")
        for split, tid, sit, prompt in (("tasks", "t-alpha", "situation alpha", "TRAIN alpha"),
                                        ("tasks", "t-beta", "situation beta", "TRAIN beta"),
                                        ("heldout", "h-zebra", "held situation zebra", "HELDOUT zebra prompt")):
            os.makedirs(os.path.join(cls.tasks_root, split), exist_ok=True)
            with open(os.path.join(cls.tasks_root, split, tid + ".toml"), "w") as f:
                f.write(TASK.format(situation=sit, worker=tid, prompt=prompt))
        cls.candidates = os.path.join(cls.root, "candidates.md")
        with open(cls.candidates, "w") as f:
            f.write("1. a candidate\n")
        # The baseline: every task, three runs, with the base marker.
        cls.baseline = os.path.join(cls.root, "baseline")
        repo = cls.make_repo(cls, "base")
        cls.set_env(FAKE_REPO=repo, FAKE_STATE=cls.state, FAKE_NEXT="", FAKE_UTIL="0.1")
        tasks = hillclimb.load_tasks(cls.tasks_root)
        binary = os.path.join(cls.root, "cases-base")
        fake_build(repo)(binary)
        plugin = os.path.join(cls.root, "plugin-base")
        hillclimb.suite.make_plugin(binary, plugin)
        os.makedirs(cls.baseline)
        runs, complete = hillclimb.evaluate(tasks, binary=binary, plugin=plugin, claude=cls.claude, model="m",
                                            effort="low", runs=3, concurrency=9, out_dir=cls.baseline,
                                            budget=hillclimb.Budget(100, None))
        assert complete

    @classmethod
    def set_env(cls, **env):
        with open(os.path.join(cls.root, "fake-env.json"), "w") as f:
            json.dump(env, f)

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.root)

    def make_repo(self, name):
        repo = os.path.join(self.root, "repo-" + name)
        os.makedirs(os.path.join(repo, "crates/cli/skill"))
        with open(os.path.join(repo, "crates/cli/skill/SKILL.md"), "w") as f:
            f.write("# cases\n\n<!-- fake: " + self.BASE + " -->\n\nThe skill.\n")
        with open(os.path.join(repo, "crates/cli/skill/REFERENCE.md"), "w") as f:
            f.write("# reference\n")
        for cmd in (["init", "-q", "-b", "hillclimb"], ["add", "."],
                    ["-c", "user.name=t", "-c", "user.email=t@example.invalid", "commit", "-q", "-m", "init"]):
            subprocess.run(["git", *cmd], cwd=repo, check=True)
        subprocess.run(["git", "config", "user.name", "t"], cwd=repo, check=True)
        subprocess.run(["git", "config", "user.email", "t@example.invalid"], cwd=repo, check=True)
        return repo

    def climb(self, name, next_marker, dry_run=False, util="0.1", **kw):
        repo = self.make_repo(name)
        self.set_env(FAKE_REPO=repo, FAKE_STATE=self.state, FAKE_NEXT=next_marker, FAKE_UTIL=util)
        a = argparse.Namespace(baseline=self.baseline, rounds=kw.get("rounds", 1), patience=2, runs=3, model="m", effort="low",
                               proposer_model="p", concurrency=kw.get("concurrency", 9), max_sessions=kw.get("max_sessions", 100),
                               stop_above=0.9, min_noise=0.05, candidates=self.candidates, claude=self.claude,
                               dry_run=dry_run)
        out = os.path.join(self.root, "out-" + name)
        code = hillclimb.hillclimb(a, build=fake_build(repo), check=lambda repo: None, repo=repo,
                                   tasks_root=self.tasks_root, out=out)
        rounds = []
        if os.path.exists(os.path.join(out, "rounds.json")):
            with open(os.path.join(out, "rounds.json")) as f:
                rounds = json.load(f)
        log = subprocess.run(["git", "log", "--format=%s"], cwd=repo, capture_output=True, text=True).stdout
        with open(os.path.join(repo, "crates/cli/skill/SKILL.md")) as f:
            skill = f.read()
        return code, rounds, log, skill, out

    def assertVerdict(self, rounds, verdict, kind):
        self.assertEqual(len(rounds), 1, rounds)
        self.assertEqual((rounds[0]["verdict"], rounds[0]["kind"]), (verdict, kind), rounds[0]["reason"])

    def test_both_improve_keeps_and_commits(self):
        code, rounds, log, skill, out = self.climb("gain", "train=111 held=111 calls=4")
        self.assertVerdict(rounds, "keep", "gain")
        self.assertIn("set the marker to train=111", log.splitlines()[0])
        self.assertIn("train=111", skill)
        with open(os.path.join(out, "report.md")) as f:
            self.assertIn("Held out before", subprocess.run(
                ["git", "log", "-1", "--format=%b"], cwd=os.path.join(self.root, "repo-gain"), capture_output=True,
                text=True).stdout)
            self.assertIn("baseline", f.read())

    def test_train_only_is_overfitting_and_reverts(self):
        code, rounds, log, skill, out = self.climb("overfit", "train=111 held=100 calls=4")
        self.assertVerdict(rounds, "revert", "overfit-heldout-dip-in-noise")
        self.assertIn(self.BASE, skill)
        self.assertEqual(log.splitlines(), ["init"])
        with open(os.path.join(out, "report.md")) as f:
            self.assertIn("(overfit-heldout-dip-in-noise): 1 of 1 rounds", f.read())

    def test_a_rule_regression_reverts(self):
        code, rounds, log, skill, _ = self.climb("regression", "train=111 held=110 calls=4 human=1")
        self.assertVerdict(rounds, "revert", "regression")
        self.assertIn("no_human_writes", rounds[0]["reason"])
        self.assertIn(self.BASE, skill)

    def test_inside_noise_reverts(self):
        code, rounds, log, skill, out = self.climb("noise", "train=000 held=110 calls=4 ")
        self.assertVerdict(rounds, "revert", "noise")
        self.assertEqual(log.splitlines(), ["init"])

    def test_a_cost_only_win_keeps(self):
        code, rounds, log, skill, _ = self.climb("cost", "train=000 held=110 calls=2")
        self.assertVerdict(rounds, "keep", "cost")
        self.assertIn("calls=2", skill)

    def test_a_plateau_stops_and_writes_the_root_causes(self):
        code, rounds, log, skill, out = self.climb("plateau", "train=000 held=110 calls=4 ", rounds=5)
        self.assertEqual([r["kind"] for r in rounds], ["noise", "noise"])
        # Round 2's proposer sees round 1, but not its held-out numbers.
        with open(os.path.join(out, "round-2", "proposer-input.md")) as f:
            history = f.read().split("## Earlier rounds")[1].split("## crates/")[0]
        self.assertIn("round 1:", history)
        self.assertIn("training score", history)
        self.assertNotIn("held out", history)
        self.assertNotIn("heldout", history)
        self.assertIn("reverted", history)
        with open(os.path.join(out, "root-causes.md")) as f:
            note = f.read()
        self.assertIn("## skill defect", note)
        self.assertIn("revision_on_note_close", note)
        with open(os.path.join(out, "report.md")) as f:
            self.assertIn("Nothing to merge", f.read())

    def test_the_proposer_sees_nothing_held_out(self):
        self.climb("leak", "train=111 held=111 calls=4")
        with open(os.path.join(self.state, "proposer-input.txt")) as f:
            prompt = f.read()
        for needle in ("h-zebra", "zebra", "heldout", "held out", "HELDOUT", self.tasks_root):
            self.assertNotIn(needle, prompt)
        self.assertIn("situation alpha", prompt)

    def test_the_rate_limit_stops_the_round_and_reverts(self):
        # One run at a time, so the stop lands with runs still to start.
        code, rounds, log, skill, _ = self.climb("ratelimit", "train=111 held=111 calls=4", util="0.95",
                                                 concurrency=1)
        self.assertEqual(rounds[0]["kind"], "stopped")
        self.assertIn("utilisation 0.95", rounds[0]["reason"])
        self.assertIn(self.BASE, skill)

    def test_the_cap_stops_before_a_round(self):
        code, rounds, log, skill, out = self.climb("cap", "train=111 held=111 calls=4", max_sessions=5)
        self.assertEqual(rounds, [])
        with open(os.path.join(out, "report.md")) as f:
            self.assertIn("would pass the cap", f.read())

    def test_dry_run_counts_sessions_and_starts_none(self):
        before = len(os.listdir(self.state))
        from io import StringIO
        from contextlib import redirect_stdout
        buf = StringIO()
        with redirect_stdout(buf):
            code, rounds, *_ = self.climb("dry", "train=111 held=111 calls=4", dry_run=True)
        self.assertEqual(code, 0, buf.getvalue())
        self.assertIn("Per round: 1 proposer + 6 train + 3 held-out runs = 10 sessions", buf.getvalue())
        self.assertIn("Dry run: no session started.", buf.getvalue())
        self.assertEqual(len(os.listdir(self.state)), before)


class Decide(unittest.TestCase):
    """The rule not reachable with the fake's patterns: a held-out drop beyond noise."""

    def m(self, score, rules=None, calls=4.0):
        return {"score": score, "rules": rules or {}, "cost": {"calls": calls, "tokens": 1000.0}}

    def test_a_heldout_drop_beyond_noise_reverts(self):
        noise = {s: {"score": 0.05, "rules": {}, "cost": {"calls": 0.1, "tokens": 0.1}} for s in ("tasks", "heldout")}
        inc = {"tasks": self.m(0.8), "heldout": self.m(0.8)}
        cand = {"tasks": self.m(0.95), "heldout": self.m(0.7)}
        self.assertEqual(hillclimb.decide(inc, cand, noise)[:2], ("revert", "heldout-drop"))

    def test_a_fall_of_exactly_the_floor_is_not_a_regression(self):
        # 22/24 against 23/24 with a floor of 1/24: the division rounds 22/24
        # below 23/24 - 1/24, which must not count as beyond the floor.
        noise = {s: {"score": 0.05, "rules": {"r": 1 / 24}, "cost": {"calls": 0.1, "tokens": 0.1}}
                 for s in ("tasks", "heldout")}
        inc = {"tasks": self.m(0.8), "heldout": self.m(0.8, {"r": (23, 24)})}
        cand = {"tasks": self.m(0.8), "heldout": self.m(0.8, {"r": (22, 24)})}
        self.assertNotEqual(hillclimb.decide(inc, cand, noise)[1], "regression")
        cand["heldout"] = self.m(0.8, {"r": (21, 24)})
        self.assertEqual(hillclimb.decide(inc, cand, noise)[1], "regression")


class ApplyDiff(unittest.TestCase):
    def setUp(self):
        self.repo = tempfile.mkdtemp()
        os.makedirs(os.path.join(self.repo, "crates/cli/skill"))
        for name in ("crates/cli/skill/SKILL.md", "Cargo.toml"):
            with open(os.path.join(self.repo, name), "w") as f:
                f.write("a\n")
        subprocess.run(["git", "init", "-q"], cwd=self.repo, check=True)
        subprocess.run(["git", "add", "."], cwd=self.repo, check=True)

    def tearDown(self):
        shutil.rmtree(self.repo)

    SKILL_HUNK = ("diff --git a/crates/cli/skill/SKILL.md b/crates/cli/skill/SKILL.md\n"
                  "--- a/crates/cli/skill/SKILL.md\n+++ b/crates/cli/skill/SKILL.md\n@@ -1 +1 @@\n-a\n+b\n")

    def test_a_skill_only_diff_applies(self):
        hillclimb.apply_diff(self.repo, self.SKILL_HUNK)
        with open(os.path.join(self.repo, "crates/cli/skill/SKILL.md")) as f:
            self.assertEqual(f.read(), "b\n")

    def test_a_diff_with_no_content_lines_for_another_file_is_refused(self):
        # A mode change or an empty new file has no ---/+++ lines.
        for extra in ("diff --git a/Cargo.toml b/Cargo.toml\nold mode 100644\nnew mode 100755\n",
                      "diff --git a/evil b/evil\nnew file mode 100644\nindex 0000000..e69de29\n"):
            with self.assertRaises(ValueError):
                hillclimb.apply_diff(self.repo, self.SKILL_HUNK + extra)
        self.assertFalse(os.path.exists(os.path.join(self.repo, "evil")))


class RateLimitPeak(unittest.TestCase):
    """The utilisation --stop-above reads from a session's transcript."""

    def peak(self, *infos):
        d = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, d)
        path = os.path.join(d, "transcript.jsonl")
        with open(path, "w", encoding="utf-8") as f:
            f.write(json.dumps({"type": "assistant", "message": {}}) + "\n")
            for info in infos:
                f.write(json.dumps({"type": "rate_limit_event", "rate_limit_info": info}) + "\n")
        return hillclimb.rate_limit_peak(path)

    def test_a_window_under_unified_windows_gives_its_utilisation(self):
        info = {"status": "allowed", "rateLimitType": "seven_day",
                "unifiedWindows": {"seven_day": {"utilization": 0.32, "status": "allowed"}}}
        self.assertEqual(self.peak(info), (0.32, False))

    def test_two_windows_give_the_higher(self):
        info = {"status": "allowed", "unifiedWindows": {
            "five_hour": {"utilization": 0.91, "status": "allowed"},
            "seven_day": {"utilization": 0.34, "status": "allowed"}}}
        self.assertEqual(self.peak(info, {"status": "allowed", "utilization": 0.5}), (0.91, False))

    def test_the_old_top_level_form_still_works(self):
        self.assertEqual(self.peak({"status": "allowed", "utilization": 0.4},
                                   {"status": "rejected", "utilization": 0.2}), (0.4, True))

    def test_a_rejected_window_is_refused(self):
        info = {"status": "allowed", "unifiedWindows": {
            "seven_day": {"utilization": 0.3, "status": "allowed"},
            "five_hour": {"utilization": 1.0, "status": "rejected"}}}
        self.assertEqual(self.peak(info), (1.0, True))

    def test_windows_not_keyed_by_name_are_skipped(self):
        self.assertEqual(self.peak({"utilization": 0.2, "unifiedWindows": [{"utilization": 0.5}]}), (0.2, False))

    def test_no_transcript_reads_nothing(self):
        self.assertEqual(hillclimb.rate_limit_peak("/nonexistent/transcript.jsonl"), (0.0, False))


if __name__ == "__main__":
    unittest.main()
