#!/usr/bin/env python3
"""Runs the cases skill's eval suite, grades it and writes a summary.

    python3 evals/cases-skill/run.py [run] [--split train|heldout|all] [--task GLOB]
        [--arms with,without] [--runs 3] [--model claude-sonnet-5-5] [--effort low]
        [-j 4] [--no-judge] [--skill-file PATH] [--out DIR] [--max-sessions N]
    python3 evals/cases-skill/run.py grade RESULTS_DIR [--no-judge]
    python3 evals/cases-skill/run.py report RESULTS_DIR

`run` builds this checkout's `cases`, runs each task in each arm, grades the
runs and writes summary.md and summary.json into the results directory
(evals/cases-skill/results/<timestamp>/, gitignored). `grade` re-grades a
results directory and says where the grades differ from the last grading.
The held-out tasks run only with --split heldout or all. Needs python 3.11+
and a logged-in `claude`.
"""
import argparse
import concurrent.futures
import datetime
import fnmatch
import json
import os
import shutil
import subprocess
import sys
import threading
import tomllib

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
sys.path.insert(0, os.path.join(HERE, "..", "harness"))

import graders  # noqa: E402
import judge as judge_mod  # noqa: E402
import report  # noqa: E402
from runner import base_env, rate_limit_peak, run_task  # noqa: E402

CLAIMS = [
    ("one_question", "Every case the agent opened asks exactly one question (a decision's options, or an "
                     "approval's rows, are one question)."),
    ("phone_readable", "Every case body could be understood on a phone by someone with no other context: it "
                       "says what the agent is doing and what it needs from the human."),
    ("options_explained", "Every decision case says what its options cost or imply and why the agent "
                          "recommends the one it does (null if there is no decision case)."),
    ("outcome_honest", "Every close outcome says what the agent actually did, as the commands show, and it "
                       "matches the human's answer, such as only approved rows run (null if nothing was closed)."),
]


def load_tasks(split, pattern):
    dirs = {"train": ["tasks"], "heldout": ["heldout"], "all": ["tasks", "heldout"]}[split]
    tasks = []
    for d in dirs:
        path = os.path.join(HERE, d)
        for name in sorted(os.listdir(path)):
            if not name.endswith(".toml"):
                continue
            with open(os.path.join(path, name), "rb") as f:
                task = tomllib.load(f)
            task.setdefault("id", name[:-5])
            task["split"] = d
            if fnmatch.fnmatch(task["id"], pattern):
                tasks.append(task)
    return tasks


def build(binary_out):
    subprocess.run(["cargo", "build", "--release", "--locked", "-p", "cases"], cwd=ROOT, check=True)
    shutil.copy2(os.path.join(ROOT, "target", "release", "cases"), binary_out)


def make_plugin(binary, dest, skill_file=None):
    """A plugin holding only the skill, as `cases skill install` writes it."""
    skill_dir = os.path.join(dest, "skills", "cases")
    os.makedirs(skill_dir, exist_ok=True)
    os.makedirs(os.path.join(dest, ".claude-plugin"), exist_ok=True)
    with open(os.path.join(dest, ".claude-plugin", "plugin.json"), "w") as f:
        json.dump({"name": "cases-skill", "version": "0.0.0",
                   "description": "The cases skill, under evaluation"}, f)
    text = subprocess.run([binary, "skill", "show"], capture_output=True, text=True, check=True).stdout
    if skill_file:
        head = text[: text.index("\n---\n", 4) + 5]
        text = head + "\n" + open(skill_file, encoding="utf-8").read()
    with open(os.path.join(skill_dir, "SKILL.md"), "w", encoding="utf-8") as f:
        f.write(text)


def real_store_count(binary, workers):
    """How many cases the real store holds from the suite's worker names. It
    only reads. The suite never points at the real store, so the count must
    not change across a run."""
    env = {k: os.environ[k] for k in ("HOME", "USER", "PATH") if k in os.environ}
    argv = [binary, "list", "--all", "--count"]
    for w in workers:
        argv += ["--worker", w]
    p = subprocess.run(argv, env=env, capture_output=True, text=True)
    return p.stdout.strip() if p.returncode == 0 else "unreadable: " + p.stderr.strip()


def judge_items(out_dir, graded):
    items = []
    for g in graded:
        if g.get("infra_error") or g.get("cost") is None:
            continue
        run_dir = os.path.join(out_dir, "runs", g["run"])
        store = json.load(open(os.path.join(run_dir, "store.json")))
        info = json.load(open(os.path.join(run_dir, "run.json")))
        seeded = set(info.get("seeded") or [])
        cases = []
        for c in store:
            if (c.get("actor") or {}).get("kind") != "agent" or c["id"] in seeded:
                continue
            cases.append({
                "kind": c["kind"], "title": c["title"], "body": (c.get("body") or "")[:3000],
                "options": c.get("options"), "recommend": c.get("recommend"),
                "rows": c.get("rows"), "answers": [e["data"] for e in c["events"]
                                                   if e["author"] == "human"],
                "close_outcome": next((e["data"].get("outcome") for e in c["events"] if e["event"] == "close"), None),
            })
        t = graders.Transcript(os.path.join(run_dir, "transcript.jsonl"))
        cmds = [x.cmd[:300] for x in t.tools if x.name == "Bash"][-25:]
        items.append({"key": g["run"], "cases": cases, "agent_commands": cmds})
    return items


def grade_dir(out_dir, use_judge, claude, judge_limit=None):
    """Grades every run in out_dir. judge_limit, a number of runs, re-judges
    only the first runs and keeps the last verdicts for the rest."""
    meta = json.load(open(os.path.join(out_dir, "meta.json")))
    tasks = {t["id"]: t for t in load_tasks("all", "*")}
    graded = []
    for name in sorted(os.listdir(os.path.join(out_dir, "runs"))):
        run_dir = os.path.join(out_dir, "runs", name)
        if not os.path.exists(os.path.join(run_dir, "run.json")):
            continue
        info = json.load(open(os.path.join(run_dir, "run.json")))
        if info.get("infra_error") and not os.path.exists(os.path.join(run_dir, "store.json")):
            # The run broke before the session left anything to grade.
            graded.append({"task": info["task"], "arm": info["arm"], "run": name,
                           "infra_error": info["infra_error"], "rules": {}, "cost": None})
            continue
        try:
            graded.append(graders.grade(run_dir, tasks[info["task"]]))
        except Exception as e:
            graded.append({"task": info["task"], "arm": info["arm"], "run": name,
                           "infra_error": f"grader crashed: {type(e).__name__}: {e}", "rules": {}, "cost": None})
    previous = os.path.join(out_dir, "graded.json")
    old = {}
    if os.path.exists(previous):
        old = {g["run"]: g for g in json.load(open(previous))}
        diffs = [(g["run"], k) for g in graded for k, v in g["rules"].items()
                 if g["run"] in old and old[g["run"]]["rules"].get(k, {}).get("result") != v["result"]]
        meta["regrade_diffs"] = diffs
        print(f"re-grade: {len(diffs)} rule verdicts differ from the last grading")
    # Runs the judge does not see this time keep its last verdicts.
    for g in graded:
        if old.get(g["run"], {}).get("judge"):
            g["judge"] = old[g["run"]]["judge"]
    if use_judge:
        items = judge_items(out_dir, graded)
        if judge_limit is not None:
            items = items[:judge_limit]
        verdicts, sessions = judge_mod.judge(items, CLAIMS, claude=claude, log_dir=out_dir)
        meta["judge_sessions"] = meta.get("judge_sessions", 0) + sessions
        for g in graded:
            if g["run"] in verdicts:
                g["judge"] = verdicts[g["run"]]
        if os.path.exists(previous):
            old = {g["run"]: g.get("judge") or {} for g in json.load(open(previous))}
            same = total = 0
            for g in graded:
                if g["run"] not in verdicts:
                    continue
                for c, v in (g.get("judge") or {}).items():
                    before = old.get(g["run"], {}).get(c)
                    if before is not None:
                        total += 1
                        same += before.get("v") == v.get("v")
            if total:
                meta["judge_agreement"] = f"{same}/{total} claim verdicts the same"
    with open(previous, "w") as f:
        json.dump(graded, f, indent=1)
    with open(os.path.join(out_dir, "meta.json"), "w") as f:
        json.dump(meta, f, indent=1)
    report.write(out_dir, graded, meta, tasks)
    return graded


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cmd", nargs="?", default="run", choices=["run", "grade", "report", "baseline"])
    ap.add_argument("dir", nargs="?")
    ap.add_argument("--split", default="train", choices=["train", "heldout", "all"])
    ap.add_argument("--task", default="*")
    ap.add_argument("--arms", default="with,without")
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--model", default="claude-sonnet-5-5")
    ap.add_argument("--effort", default="low")
    ap.add_argument("-j", "--concurrency", type=int, default=4)
    ap.add_argument("--no-judge", action="store_true")
    ap.add_argument("--skill-file")
    ap.add_argument("--out")
    ap.add_argument("--max-sessions", type=int, default=250,
                    help="refuse to start when the run needs more claude sessions than this")
    ap.add_argument("--claude", default=shutil.which("claude"))
    ap.add_argument("--resume", action="store_true",
                    help="with --out: skip the runs that already finished, rerun the rest")
    ap.add_argument("--stop-above", type=float,
                    help="start no more sessions once the account reports a utilisation above this (0..1)")
    ap.add_argument("--judge-limit", type=int, help="grade: re-judge only this many runs")
    ap.add_argument("--dry-run", action="store_true", help="check everything, start no session")
    a = ap.parse_args()

    if a.cmd == "grade":
        grade_dir(a.dir, not a.no_judge, a.claude, a.judge_limit)
        return
    if a.cmd == "baseline":
        write_baseline(a.dir)
        return
    if a.cmd == "report":
        graded = json.load(open(os.path.join(a.dir, "graded.json")))
        meta = json.load(open(os.path.join(a.dir, "meta.json")))
        report.write(a.dir, graded, meta, {t["id"]: t for t in load_tasks("all", "*")})
        return

    tasks = load_tasks(a.split, a.task)
    arms = a.arms.split(",")
    jobs = [(t, arm, k) for t in tasks for arm in arms for k in range(a.runs)]
    if len(jobs) > a.max_sessions and not a.dry_run:
        sys.exit(f"{len(jobs)} sessions is more than --max-sessions {a.max_sessions}")
    stamp = datetime.datetime.now().strftime("%Y%m%dT%H%M%S")
    if a.dry_run:
        dry_run(a, tasks, arms, jobs)
        return
    out_dir = os.path.abspath(a.out or os.path.join(HERE, "results", stamp))
    if os.path.exists(os.path.join(out_dir, "meta.json")) and not a.resume:
        sys.exit(f"{out_dir} holds a run already: pass --resume to finish it")
    os.makedirs(os.path.join(out_dir, "runs"), exist_ok=True)
    binary = os.path.join(out_dir, "cases")
    build(binary)
    plugin = os.path.join(out_dir, "plugin")
    make_plugin(binary, plugin, a.skill_file)
    workers = sorted({t["worker"] for t in tasks if t.get("worker")})
    meta_path = os.path.join(out_dir, "meta.json")
    if a.resume and os.path.exists(meta_path):
        meta = json.load(open(meta_path))
        before = meta["real_store_before"]
        meta.setdefault("resumed", []).append(stamp)
    else:
        before = real_store_count(binary, workers)
        meta = {"started": stamp, "split": a.split, "tasks": [t["id"] for t in tasks], "arms": arms,
                "runs": a.runs, "model": a.model, "effort": a.effort, "sessions": 0,
                "skill_file": a.skill_file, "real_store_before": before,
                "commit": subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True,
                                         text=True).stdout.strip()}
    todo = [j for j in jobs if not finished(os.path.join(out_dir, "runs", job_name(j)))]
    with open(meta_path, "w") as f:
        json.dump(meta, f, indent=1)
    print(f"{len(todo)} runs to do of {len(jobs)}, into {out_dir}")
    stop = threading.Event()
    lock = threading.Lock()

    def one(job):
        if stop.is_set():
            return
        name = job_name(job)
        task, arm, _ = job
        run_dir = os.path.join(out_dir, "runs", name)
        shutil.rmtree(run_dir, ignore_errors=True)
        info = run_task(task, arm=arm, binary=binary, claude=a.claude, plugin_dir=plugin, model=a.model,
                        effort=a.effort, out_dir=run_dir)
        with lock:
            meta["sessions"] += 1
        flag = info["infra_error"] or ("timed out" if info["timed_out"] else "")
        peak, refused = rate_limit_peak(os.path.join(run_dir, "transcript.jsonl"))
        print(f"{name}: {info['seconds']}s {flag} (utilisation {peak:.2f})", flush=True)
        if a.stop_above is not None and (refused or peak > a.stop_above):
            if not stop.is_set():
                print(f"STOP: the account reports utilisation {peak:.2f}"
                      f"{' and refused a request' if refused else ''}, above {a.stop_above}", flush=True)
            stop.set()

    with concurrent.futures.ThreadPoolExecutor(a.concurrency) as pool:
        list(pool.map(one, todo))
    meta["real_store_after"] = real_store_count(binary, workers)
    if meta["real_store_after"] != before:
        print(f"WARNING: the real store's count of eval-worker cases changed: {before} -> {meta['real_store_after']}")
    left = [job_name(j) for j in jobs if not finished(os.path.join(out_dir, "runs", job_name(j)))]
    meta["unfinished"] = left
    with open(meta_path, "w") as f:
        json.dump(meta, f, indent=1)
    if left:
        # Grading a partial run would give a partial baseline: stop here and resume later.
        sys.exit(f"{len(left)} runs not finished; rerun with --out {out_dir} --resume")
    grade_dir(out_dir, not a.no_judge, a.claude)
    print(os.path.join(out_dir, "summary.md"))


def job_name(job):
    task, arm, k = job
    return f"{task['id']}.{arm}.{k}"


def finished(run_dir):
    """A run that ended without a harness error; others are run again on --resume."""
    path = os.path.join(run_dir, "run.json")
    if not os.path.exists(path):
        return False
    with open(path) as f:
        return not json.load(f).get("infra_error")


def dry_run(a, tasks, arms, jobs):
    """Checks what a run needs and lists what it would do. Starts no session."""
    import math
    import tempfile
    from runner import Store, make_env, session_argv
    ok = True

    def check(name, good, detail=""):
        nonlocal ok
        ok = ok and good
        print(f"[{'ok' if good else 'FAIL'}] {name}{': ' + detail if detail else ''}")

    batches = math.ceil(len(jobs) / judge_mod.BATCH) if not a.no_judge else 0
    print(f"Agent runs: {len(jobs)} ({len(tasks)} tasks x {len(arms)} arms x {a.runs}), "
          f"model {a.model}, effort {a.effort}, -j {a.concurrency}")
    for j in jobs:
        print("  " + job_name(j))
    print(f"Judge: {batches} batches of up to {judge_mod.BATCH} runs ({judge_mod.MODEL}), "
          f"then {math.ceil(CONSISTENCY_RUNS / judge_mod.BATCH)} to re-judge {CONSISTENCY_RUNS} runs for consistency")
    check("session budget", len(jobs) <= a.max_sessions, f"{len(jobs)} of at most {a.max_sessions}")
    tmp = tempfile.mkdtemp(prefix="cases-eval-dry-")
    binary = os.path.join(tmp, "cases")
    try:
        build(binary)
        version = subprocess.run([binary, "--version"], capture_output=True, text=True).stdout.strip()
        check("binary builds from this checkout", bool(version), version)
        plugin = os.path.join(tmp, "plugin")
        make_plugin(binary, plugin, a.skill_file)
        with open(os.path.join(plugin, "skills", "cases", "SKILL.md")) as f:
            head = f.read(200)
        check("skill plugin", head.startswith("---\nname: cases\n"), os.path.join("skills", "cases", "SKILL.md"))
        workers = sorted({t["worker"] for t in tasks if t.get("worker")})
        real_before = real_store_count(binary, workers)
        ws = os.path.join(tmp, "ws")
        env = make_env(ws, tmp)
        store = Store(binary, env["CASES_STORE"], env)
        p = store.run(["open", "--kind", "fyi", "--urgency", "whenever", "--title", "dry run", "--worker",
                       "eval-dry-run"])
        in_ws = os.path.exists(env["CASES_STORE"]) and env["CASES_STORE"].startswith(ws)
        check("a case opened with the run's environment lands in its workspace store",
              p.returncode == 0 and in_ws and len(store.cases()) == 1, env["CASES_STORE"])
        # Where the store resolves, with the run's environment and with
        # CASES_STORE dropped: both must be inside the workspace.
        for label, e in (("CASES_STORE", env), ("XDG fallback", {k: v for k, v in env.items() if k != "CASES_STORE"})):
            shown = subprocess.run([binary, "config", "show"], env=e, capture_output=True, text=True).stdout
            line = next((x.split()[1] for x in shown.splitlines() if x.strip().startswith("store")), "")
            check(f"store resolves inside the workspace ({label})", line.startswith(ws), line)
        check("real store: cases from the suite's worker names (read only)",
              real_store_count(binary, workers) == real_before, real_before)
        argv = session_argv(a.claude or "claude", arm="with", model=a.model, effort=a.effort,
                            plugin_dir=plugin)
        isolated = ("--setting-sources" in argv and argv[argv.index("--setting-sources") + 1] == "project,local"
                    and '"enabled": true' in argv[argv.index("--settings") + 1])
        check("session loads only project and local settings, with the sandbox on", isolated)
        if sys.platform.startswith("linux"):
            missing = [t for t in ("bwrap", "socat") if not shutil.which(t)]
            check("the Linux sandbox's tools are installed (bwrap, socat)", not missing,
                  "missing: " + ", ".join(missing) if missing else "")
        auth = subprocess.run([a.claude or "claude", "auth", "status", "--json"], capture_output=True, text=True,
                              env=base_env("HOME", "USER", "PATH"))
        try:
            st = json.loads(auth.stdout)
            check("claude is logged in", bool(st.get("loggedIn")),
                  f"{st.get('authMethod')}, {st.get('subscriptionType')}")
        except json.JSONDecodeError:
            check("claude is logged in", False, auth.stderr.strip()[:200])
        check("gh is logged in", subprocess.run(["gh", "auth", "status"], capture_output=True).returncode == 0)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    print("Dry run: no session started." + ("" if ok else " Some checks FAILED."))
    if not ok:
        sys.exit(1)


CONSISTENCY_RUNS = 40


def write_baseline(out_dir):
    """BASELINE.md from a graded results directory: the summary, and what it rests on."""
    meta = json.load(open(os.path.join(out_dir, "meta.json")))
    with open(os.path.join(out_dir, "summary.md")) as f:
        summary = f.read().split("\n", 1)[1]
    text = f"""# Baseline of the cases skill eval

The first measured run of the suite in `evals/cases-skill/`, on commit `{meta.get('commit', '')[:10]}`, before any change to the skill. A hillclimb compares against these numbers: the training split to improve on, the held-out split as its final check. Raw results are not committed; this file is `run.py baseline` over them.

What the numbers rest on, and where the graders are weak, is in the suite's README ("Graders" and "Known weaknesses of the graders"). Hillclimb candidate #1, seen before this run: SKILL.md says to scope every `wait --pickup` with `--id` or with `--label` and `--worker`, but its messages section describes a listening wait with `--worker` alone. The grader accepts `--worker` alone.
{summary}"""
    path = os.path.join(HERE, "BASELINE.md")
    with open(path, "w") as f:
        f.write(text)
    print(path)


if __name__ == "__main__":
    main()
