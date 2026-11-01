#!/usr/bin/env python3
"""Hillclimbs the cases skill (SKILL.md and REFERENCE.md) against the eval suite.

    python3 evals/cases-skill/hillclimb.py --baseline RESULTS_DIR [--rounds 5] [--patience 2]
        [--runs 3] [-j 4] [--max-sessions 550] [--stop-above 0.90] [--dry-run]

Each round:

1. It summarises the incumbent's failures on the training tasks, by rule, from
   grades only: counts, the situations they fell in, cost. No transcript text
   and nothing from the held-out tasks goes into it.
2. A proposer (one `claude -p` session, no tools) gets that summary, the
   skill's two files, the seed candidates and the earlier rounds, and returns
   one diff with a one-line hypothesis.
3. The driver applies the diff to the skill files only, rebuilds `cases` (the
   skill is embedded in it), and runs the training and held-out tasks with
   the skill, `--runs` times each.
4. It keeps or reverts the patch by `decide()`, against noise floors taken
   from the baseline's run-to-run variance. A kept patch is its own commit.

It stops after --rounds, after --patience rounds in a row with nothing kept
(and then writes a root-cause note on the failures left), when the next
round would pass --max-sessions, or when the account reports a rate limit
past --stop-above. It writes report.md in its results directory.

Run it on a branch of its own with a clean tree: it commits there.
"""
import argparse
import concurrent.futures
import datetime
import json
import math
import os
import re
import shutil
import subprocess
import sys
import threading
import tomllib

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "harness"))

import graders  # noqa: E402
import run as suite  # noqa: E402
from runner import run_task  # noqa: E402
from stats import fmt_rate, mean  # noqa: E402

SKILL_FILES = ("crates/cli/skill/SKILL.md", "crates/cli/skill/REFERENCE.md")
Z = 2.0  # noise floors are Z standard errors of a difference between two runs of the suite
EPS = 1e-9  # a score difference smaller than this is float rounding, not a change


# ---------------------------------------------------------------- tasks and runs

def load_tasks(root):
    """Training tasks from root/tasks, held-out tasks from root/heldout."""
    tasks = []
    for split in ("tasks", "heldout"):
        path = os.path.join(root, split)
        for name in sorted(os.listdir(path)) if os.path.isdir(path) else []:
            if name.endswith(".toml"):
                with open(os.path.join(path, name), "rb") as f:
                    task = tomllib.load(f)
                task.setdefault("id", name[:-5])
                task["split"] = split
                tasks.append(task)
    return tasks


def rate_limit_peak(transcript_path):
    """The highest utilisation the account reported in a session, and whether a request was refused."""
    peak, refused = 0.0, False
    if not os.path.exists(transcript_path):
        return peak, refused
    with open(transcript_path, encoding="utf-8") as f:
        for line in f:
            if '"rate_limit_event"' not in line:
                continue
            try:
                info = json.loads(line).get("rate_limit_info") or {}
            except json.JSONDecodeError:
                continue
            peak = max(peak, float(info.get("utilization") or 0))
            refused = refused or info.get("status") == "rejected"
    return peak, refused


class Budget:
    """Counts sessions, and stops new ones past the cap or the rate limit."""

    def __init__(self, cap, stop_above):
        self.cap = cap
        self.stop_above = stop_above
        self.used = 0
        self.stopped = None
        self.lock = threading.Lock()

    def spend(self, transcript_path=None):
        """Counts one session. Returns whether the account refused a request in it."""
        peak, refused = rate_limit_peak(transcript_path) if transcript_path else (0.0, False)
        with self.lock:
            self.used += 1
            if refused:
                self.stopped = self.stopped or "the account refused a request (rate limit)"
            elif self.stop_above is not None and peak > self.stop_above:
                self.stopped = self.stopped or f"the account reported utilisation {peak:.2f}, above {self.stop_above}"
            return refused

    def affords(self, n):
        return self.stopped is None and self.used + n <= self.cap


def evaluate(tasks, *, binary, plugin, claude, model, effort, runs, concurrency, out_dir, budget):
    """Runs every task `runs` times with the skill and grades each run.
    Returns the graded runs, each with its split, and whether all ran."""
    jobs = [(t, k) for t in tasks for k in range(runs)]
    graded = []
    lock = threading.Lock()

    def one(job):
        task, k = job
        if budget.stopped:
            return
        run_dir = os.path.join(out_dir, f"{task['id']}.with.{k}")
        run_task(task, arm="with", binary=binary, claude=claude, plugin_dir=plugin, model=model,
                 effort=effort, out_dir=run_dir)
        if budget.spend(os.path.join(run_dir, "transcript.jsonl")):
            return  # a run the rate limit cut short is not the skill's failure
        try:
            g = graders.grade(run_dir, task)
        except Exception as e:  # a grader or harness failure is not the skill's
            g = {"task": task["id"], "arm": "with", "run": os.path.basename(run_dir), "rules": {}, "cost": None,
                 "infra_error": f"{type(e).__name__}: {e}"}
        g["split"] = task["split"]
        with lock:
            graded.append(g)

    with concurrent.futures.ThreadPoolExecutor(concurrency) as pool:
        list(pool.map(one, jobs))
    graded.sort(key=lambda g: g["run"])
    with open(os.path.join(out_dir, "graded.json"), "w") as f:
        json.dump(graded, f, indent=1)
    return graded, len(graded) == len(jobs)


# ---------------------------------------------------------------- numbers

def run_score(g):
    vals = [v["result"] for v in g["rules"].values() if v["result"] in ("pass", "fail")]
    return sum(v == "pass" for v in vals) / len(vals) if vals else None


def run_cost(g):
    """API calls and context tokens per decision (per run when the run had none)."""
    c = g["cost"]
    d = max(c.get("decisions") or 0, 1)
    return {"calls": c["api_calls"] / d, "tokens": c["context_tokens"] / d}


def metrics(graded, split):
    """The numbers the keep/revert rules compare, for one split."""
    gs = [g for g in graded if g.get("split") == split and not g.get("infra_error") and g.get("cost")]
    rules = {}
    for g in gs:
        for rule, v in g["rules"].items():
            if v["result"] in ("pass", "fail"):
                p, n = rules.get(rule, (0, 0))
                rules[rule] = (p + (v["result"] == "pass"), n + 1)
    per_task = {}
    for g in gs:
        per_task.setdefault(g["task"], []).append(g)
    score = mean([mean([run_score(g) for g in runs]) for runs in per_task.values()])
    cost = {k: mean([mean([run_cost(g)[k] for g in runs]) for runs in per_task.values()])
            for k in ("calls", "tokens")}
    return {"runs": len(gs), "tasks": len(per_task), "score": score, "rules": rules, "cost": cost,
            "per_task": {t: [run_score(g) for g in runs] for t, runs in per_task.items()},
            "per_task_cost": {t: [run_cost(g) for g in runs] for t, runs in per_task.items()}}


def _se_of_mean(per_task_values):
    """The standard error of a suite mean (the mean of per-task means), from
    the run-to-run variance within each task."""
    terms, tasks = 0.0, 0
    for vals in per_task_values:
        vals = [v for v in vals if v is not None]
        if not vals:
            continue
        tasks += 1
        if len(vals) > 1:
            m = sum(vals) / len(vals)
            terms += sum((v - m) ** 2 for v in vals) / (len(vals) - 1) / len(vals)
    return math.sqrt(terms) / tasks if tasks else 0.0


def noise_floor(baseline, min_floor=0.0):
    """Noise floors from the baseline's run-to-run variance.

    For a split's score (the mean over tasks of each task's mean run score),
    the standard error comes from each task's variance across its runs. Two
    runs of the suite each carry that error, so a difference beyond
    Z * sqrt(2) * SE counts as real. A rule's floor is the same bound on a
    pass rate, sqrt(p(1-p)/n), and never less than one check in n. Cost
    floors use the per-run cost per decision the same way, relative to the
    baseline's mean."""
    out = {}
    for split in ("tasks", "heldout"):
        m = metrics(baseline, split)
        se = _se_of_mean(m["per_task"].values())
        out[split] = {"score": max(Z * math.sqrt(2) * se, min_floor), "rules": {}, "cost": {}}
        for rule, (p, n) in m["rules"].items():
            rate = p / n
            out[split]["rules"][rule] = max(Z * math.sqrt(2) * math.sqrt(rate * (1 - rate) / n), 1 / n, min_floor)
        for k in ("calls", "tokens"):
            se_k = _se_of_mean([[c[k] for c in runs] for runs in m["per_task_cost"].values()])
            base = m["cost"][k] or 1
            out[split]["cost"][k] = max(Z * math.sqrt(2) * se_k / base, min_floor)
    return out


def decide(inc, cand, noise):
    """Keep or revert a candidate against the incumbent.

    inc and cand map a split ("tasks", "heldout") to metrics(); noise is noise_floor().
    Returns (verdict, kind, reason), verdict being "keep" or "revert".

    1. Any rule, in either split, whose pass rate falls beyond its floor: revert.
    2. The held-out score falls beyond its floor: revert.
    3. The training score rises beyond its floor: keep if the held-out score
       did not fall at all; if it fell (inside noise), only training improved,
       which is overfitting: revert, as overfit-heldout-dip-in-noise.
    4. Scores hold (no fall beyond noise in either split) and cost per
       decision on the training tasks falls beyond its floor, calls or
       tokens, with no cost rise beyond noise on the held-out ones: keep, as a
       cost win.
    5. Otherwise the change is inside noise: revert.
    """
    for split in ("tasks", "heldout"):
        for rule, (p, n) in inc[split]["rules"].items():
            c = cand[split]["rules"].get(rule)
            if not c or not n or not c[1]:
                continue
            floor = noise[split]["rules"].get(rule, 1 / n)
            # EPS: a fall of exactly the floor, such as one check in n, is
            # not beyond it, however the division rounds.
            if c[0] / c[1] < p / n - floor - EPS:
                name = "train" if split == "tasks" else "held-out"
                return "revert", "regression", (f"{rule} fell on {name} tasks from {p}/{n} to {c[0]}/{c[1]}, "
                                                f"beyond the noise floor of {floor:.2f}")
    d_train = cand["tasks"]["score"] - inc["tasks"]["score"]
    d_held = cand["heldout"]["score"] - inc["heldout"]["score"]
    f_train, f_held = noise["tasks"]["score"], noise["heldout"]["score"]
    nums = f"train {d_train:+.3f} (floor {f_train:.3f}), held out {d_held:+.3f} (floor {f_held:.3f})"
    if d_held < -f_held:
        return "revert", "heldout-drop", "held-out score fell beyond noise: " + nums
    if d_train > f_train:
        if d_held < -EPS:
            # Strict: any held-out fall, even inside noise, means only training
            # improved. With a small held-out split this can revert a real fix,
            # so it has its own kind and the report counts it. EPS keeps float
            # rounding (the same run scores summed in another order) from
            # counting as a fall.
            return "revert", "overfit-heldout-dip-in-noise", ("only the training score rose; the held-out score "
                                                              "fell, inside noise: " + nums)
        return "keep", "gain", "training score rose beyond noise and held out did not fall: " + nums
    if d_train >= -f_train:
        cuts = []
        for k in ("calls", "tokens"):
            before, after = inc["tasks"]["cost"][k], cand["tasks"]["cost"][k]
            held_before, held_after = inc["heldout"]["cost"][k], cand["heldout"]["cost"][k]
            if before and (before - after) / before > noise["tasks"]["cost"][k]:
                if not held_before or (held_after - held_before) / held_before <= noise["heldout"]["cost"][k]:
                    cuts.append(f"{k} per decision {before:.1f} -> {after:.1f}")
        if cuts:
            return "keep", "cost", "pass rate held and " + ", ".join(cuts) + " beyond noise: " + nums
    return "revert", "noise", "no change beyond noise: " + nums


# ---------------------------------------------------------------- the proposer

RULE_TEXT = {
    "no_human_writes": "never runs the human's commands (answer, park, resume, message, --as)",
    "no_forbidden": "never runs sweep, prune, serve, service, skill install, or touches the store file",
    "wait_scoped": "every wait --pickup has --id or --worker",
    "revision_on_note_close": "every note and close passes --revision",
    "closed_all_pickedup": "closes every case it picked up",
    "no_duplicate": "never opens a second case on the same question",
    "opens_as_expected": "opens as many cases as the task needs, none when it knows the answer",
    "no_secret": "puts no secret in a case",
    "wait_in_background": "runs every wait in the background",
    "wait_since": "passes --since on every wait on a case",
    "one_pickup_wait_per_case": "never has two pickup waits on one case at once",
    "open_chained": "starts the wait in the same command as the open",
    "close_chained": "chains a close with the next open or wait, not a call of its own",
    "wait_output_read_once": "reads a wait's output once, unfiltered",
    "exit_3_4_handled": "after exit 3 or 4, reads the case with show and never repeats the write",
    "withdrawn_respected": "writes nothing to a case once it has seen it withdrawn",
    "parked_handled": "after a park, waits again with the line's next_since",
    "message_handled": "closes every message it took",
    "status_beside_first_open": "runs cases status in the same message as its first open",
    "listed_on_resume": "reads its cases before writing when a session resumes",
    "noted_unclear": "notes a case whose answer was unclear instead of guessing",
    "outcome": "did what the answers asked, in the files and the store",
}


def failure_summary(graded, train_tasks):
    """What the proposer learns about the incumbent: failures by rule on the
    training tasks, the situations they fell in, and cost. It is built from
    grades and task metadata only: no transcript text, no grader reasons
    (they quote commands), and nothing from a held-out task."""
    situation = {t["id"]: t.get("situation", "") for t in train_tasks}
    gs = [g for g in graded if g.get("task") in situation and not g.get("infra_error") and g.get("cost")]
    lines = [f"Training runs graded: {len(gs)} over {len({g['task'] for g in gs})} tasks."]
    by_rule = {}
    for g in gs:
        for rule, v in g["rules"].items():
            if v["result"] in ("pass", "fail"):
                s = by_rule.setdefault(rule, {"pass": 0, "fail": 0, "situations": {}})
                s[v["result"]] += 1
                if v["result"] == "fail":
                    sit = situation[g["task"]]
                    s["situations"][sit] = s["situations"].get(sit, 0) + 1
    lines.append("")
    lines.append("Failures by rule (failed / checked), with the situations the failures fell in:")
    ranked = sorted(by_rule.items(), key=lambda kv: -kv[1]["fail"])
    for rule, s in ranked:
        if not s["fail"]:
            continue
        sits = "; ".join(f"{k} ({n})" for k, n in sorted(s["situations"].items(), key=lambda kv: -kv[1]))
        lines.append(f"- {rule} ({RULE_TEXT.get(rule, '')}): {s['fail']}/{s['fail'] + s['pass']}. {sits}")
    clean = [r for r, s in ranked if not s["fail"]]
    lines.append("")
    lines.append("Rules with no failures: " + (", ".join(clean) or "none") + ".")
    costs = [run_cost(g) for g in gs]
    if costs:
        lines.append("")
        lines.append(f"Cost: {mean([c['calls'] for c in costs]):.1f} API calls and "
                     f"{mean([c['tokens'] for c in costs]):,.0f} context tokens per decision, on average; "
                     f"extra calls (cases-only calls other than an open) per run "
                     f"{mean([g['cost']['extra_calls'] for g in gs]):.1f}.")
    return "\n".join(lines)


PROPOSER_PROMPT = """You improve an agent skill: instructions that teach a coding agent to ask a human through the `cases` CLI.
An eval suite grades agents that use the skill on realistic tasks. Below are the skill's two files, a summary of
where agents that used the current version failed, a list of candidate problems seen by hand, and the earlier
rounds of this hillclimb with what happened to each.

Propose ONE targeted change to the skill that you expect to fix the largest group of failures, or to cut cost per
decision without losing pass rate. Rules for the change:
- Change only crates/cli/skill/SKILL.md and crates/cli/skill/REFERENCE.md.
- Fix the general behaviour. Never write a rule aimed at one task, one situation's wording, or one example; never
  paste task, prompt or transcript content into the skill.
- Keep it small: a few lines changed. SKILL.md is loaded in every session, so every added line costs every session.
- Do not repeat a change an earlier round reverted, unless you say what is different.
- Keep the skill's plain style: short sentences, no marketing words.

Reply with JSON only, no prose and no code fence:
{{"hypothesis": "one line: what you change and which failures it should fix", "diff": "a unified diff (git format, paths a/crates/cli/skill/... and b/crates/cli/skill/...) that git apply accepts"}}

## Failure summary (training tasks only)

{summary}

## Candidate problems seen by hand (you may use one, or none)

{candidates}

## Earlier rounds

{history}

## crates/cli/skill/SKILL.md

{skill}

## crates/cli/skill/REFERENCE.md

{reference}
"""


def proposer_input(repo, summary, history, candidates_path):
    def read(rel):
        with open(os.path.join(repo, rel), encoding="utf-8") as f:
            return f.read()
    with open(candidates_path, encoding="utf-8") as f:
        candidates = f.read()
    # A round's kind and reason can quote the held-out split, so the proposer
    # gets only whether the change was kept and what happened on training.
    # (Whether it was kept is the one held-out bit it cannot avoid: the kept
    # changes are in the skill it reads.)
    hist = "\n".join(f"- round {h['round']}: {h['hypothesis']} -> "
                     f"{'kept' if h['verdict'] == 'keep' else 'reverted'}: {h.get('train') or h['reason']}"
                     for h in history) or "None yet."
    return PROPOSER_PROMPT.format(summary=summary, candidates=candidates, history=hist,
                                  skill=read(SKILL_FILES[0]), reference=read(SKILL_FILES[1]))


def propose(prompt, *, claude, model, log_path):
    """One `claude -p` session with no tools. Returns (hypothesis, diff) or raises ValueError."""
    env = {k: os.environ[k] for k in ("HOME", "USER", "LOGNAME", "LANG", "PATH") if k in os.environ}
    try:
        p = subprocess.run([claude, "-p", "--model", model, "--tools", "", "--output-format", "json",
                            "--setting-sources", "project,local", "--no-session-persistence"],
                           input=prompt, capture_output=True, text=True, env=env, timeout=1800)
    except subprocess.TimeoutExpired as e:
        raise ValueError("the proposer did not reply within 30 minutes") from e
    with open(log_path, "w", encoding="utf-8") as f:
        f.write(p.stdout)
    try:
        text = json.loads(p.stdout).get("result", "")
        m = re.search(r"\{.*\}", text, re.S)
        out = json.loads(m.group(0))
        if not isinstance(out["diff"], str):
            raise TypeError("diff is not a string")
        return out["hypothesis"].strip().splitlines()[0], out["diff"]
    except (json.JSONDecodeError, AttributeError, KeyError, TypeError, IndexError) as e:
        raise ValueError(f"the proposer's reply is not the JSON asked for: {e}") from e


def apply_diff(repo, diff):
    """Applies a diff that touches only the skill's files. Raises ValueError otherwise."""
    if not diff.endswith("\n"):
        diff += "\n"
    # Ask git which paths the patch touches: a regex over ---/+++ lines misses
    # mode changes, empty new files and binary patches, which have none.
    p = subprocess.run(["git", "apply", "--recount", "--numstat", "-"], input=diff, cwd=repo,
                       capture_output=True, text=True)
    if p.returncode != 0:
        raise ValueError("git apply refused the diff: " + p.stderr.strip()[:300])
    paths = {line.split("\t", 2)[2] for line in p.stdout.splitlines() if line.count("\t") >= 2}
    if not paths or not paths <= set(SKILL_FILES):
        raise ValueError(f"the diff touches {sorted(paths) or 'nothing'}, not only the skill's files")
    for args in (["--check"], []):
        p = subprocess.run(["git", "apply", "--recount", *args, "-"], input=diff, cwd=repo,
                           capture_output=True, text=True)
        if p.returncode != 0:
            raise ValueError("git apply refused the diff: " + p.stderr.strip()[:300])


def revert(repo):
    subprocess.run(["git", "checkout", "--", *SKILL_FILES], cwd=repo, check=True)


def check_skill(repo):
    """Runs the skill's own tests (every command and flag it names exists), so
    a patch that breaks them is refused before it costs a round of sessions
    and is never committed. Raises ValueError when they fail."""
    p = subprocess.run(["cargo", "test", "--locked", "-q", "-p", "cases", "--bin", "cases", "commands::skill"], cwd=repo,
                       capture_output=True, text=True)
    if p.returncode != 0:
        raise ValueError("the skill's tests fail with the patch: " + (p.stdout + p.stderr).strip()[-300:])


def git(repo, *args):
    return subprocess.run(["git", *args], cwd=repo, capture_output=True, text=True, check=True).stdout.strip()


def numbers(m):
    return (f"score {m['score']:.3f}, {m['cost']['calls']:.1f} calls and "
            f"{m['cost']['tokens']:,.0f} context tokens per decision ({m['runs']} runs)")


def commit(repo, round_no, hypothesis, verdict_reason, before, after):
    body = [f"Hillclimb round {round_no}: {verdict_reason}.", "",
            f"Train before: {numbers(before['tasks'])}", f"Train after: {numbers(after['tasks'])}",
            f"Held out before: {numbers(before['heldout'])}", f"Held out after: {numbers(after['heldout'])}"]
    git(repo, "add", *SKILL_FILES)
    subject = hypothesis if re.match(r"^[a-z]+(\([^)]*\))?: ", hypothesis) else "docs(skill): " + hypothesis[0].lower() + hypothesis[1:]
    git(repo, "commit", "-q", "-m", subject[:100], "-m", "\n".join(body))
    return git(repo, "rev-parse", "--short", "HEAD")


# ---------------------------------------------------------------- notes and report

def root_causes(graded, baseline_outcome):
    """A first sort of the failures left on the training tasks, from grades only:
    harness bug, grader error, ambiguous task, skill defect. It is a heuristic
    and needs a person to read it."""
    classes = {"harness bug": [], "grader error": [], "ambiguous task": [], "skill defect": []}
    for g in graded:
        if g.get("split") != "tasks":
            continue
        if g.get("infra_error"):
            classes["harness bug"].append(f"{g['run']}: {g['infra_error'][:120]}")
            continue
        for rule, v in g["rules"].items():
            if v["result"] == "error":
                classes["grader error"].append(f"{g['run']}: {rule}")
            elif v["result"] == "fail":
                if g.get("human_step_failed"):
                    # The script could not play the human against what the
                    # agent did: the task allowed a reading it did not expect.
                    classes["ambiguous task"].append(f"{g['run']}: {rule} (a human step was refused)")
                elif g.get("timed_out"):
                    classes["harness bug"].append(f"{g['run']}: {rule} (timed out: the harness or the task)")
                elif rule == "outcome" and baseline_outcome.get(g["task"]) == 0.0:
                    classes["ambiguous task"].append(f"{g['run']}: outcome, failed in every baseline run too")
                else:
                    classes["skill defect"].append(f"{g['run']}: {rule}")
    lines = ["# Root causes of the failures left (training tasks)", "",
             "Sorted from grades only, by rule of thumb: a refused human step means the task allowed a reading "
             "its script did not expect; a timeout points at the harness or the task; a grader crash is a grader "
             "error; the rest are taken as skill defects. Read before acting on it.", ""]
    for name, items in classes.items():
        lines.append(f"## {name} ({len(items)})")
        lines.append("")
        lines += [f"- {x}" for x in items] or ["None."]
        lines.append("")
    return "\n".join(lines)


def diff_ci(base_vals, cand_vals):
    """The difference of two suite scores, with a 95% interval from the per-task variances."""
    se = math.sqrt(_se_of_mean(base_vals) ** 2 + _se_of_mean(cand_vals) ** 2)
    return 1.96 * se


def final_report(out_dir, baseline, final, rounds, noise, stopped, budget):
    b, f = baseline["heldout"], final["heldout"]
    d = f["score"] - b["score"]
    half = diff_ci(list(b["per_task"].values()), list(f["per_task"].values()))
    kept = [r for r in rounds if r["verdict"] == "keep"]
    if not kept:
        advice = "Nothing to merge: no round was kept."
    elif d > noise["heldout"]["score"] and d - half > 0:
        advice = "Merge: the held-out gain is beyond noise and its interval excludes zero."
    elif any(r["kind"] == "cost" for r in kept) and d >= -noise["heldout"]["score"]:
        advice = "Merge only for cost: the held-out pass rate held; check the cost rows."
    else:
        advice = "Don't merge: the held-out gain is inside noise."

    def checks(m):
        return fmt_rate(sum(x[0] for x in m["rules"].values()), sum(x[1] for x in m["rules"].values()))

    lines = ["# Hillclimb report", "", f"**{advice}**", "",
             f"Rounds: {len(rounds)}; kept: {len(kept)}; sessions: {budget.used}. Stopped: {stopped}.", ""]
    if rounds:
        lines += [f"Reverted for a held-out dip inside noise (overfit-heldout-dip-in-noise): "
                  f"{sum(r['kind'] == 'overfit-heldout-dip-in-noise' for r in rounds)} of {len(rounds)} rounds. "
                  f"With {b['tasks']} held-out tasks this rule can revert a real fix; each such round is listed "
                  "below.", ""]
    lines += ["| | baseline | final |", "| --- | --- | --- |",
             f"| held-out mean run score | {b['score']:.3f} | {f['score']:.3f} ({d:+.3f} ± {half:.3f}, 95%) |",
             f"| held-out rule checks (Wilson 95%) | {checks(b)} | {checks(f)} |",
             f"| held-out calls per decision | {b['cost']['calls']:.1f} | {f['cost']['calls']:.1f} |",
             f"| held-out context tokens per decision | {b['cost']['tokens']:,.0f} | {f['cost']['tokens']:,.0f} |",
             f"| train mean run score | {baseline['tasks']['score']:.3f} | {final['tasks']['score']:.3f} |",
             "", "The held-out runs took part in every keep decision, so the final held-out delta is an "
             "optimistic estimate: confirm a merge with a fresh `run.py --split heldout` run of the final skill.",
             "", f"Noise floors (from the baseline): train {noise['tasks']['score']:.3f}, "
             f"held out {noise['heldout']['score']:.3f}.", "", "## Rounds", ""]
    for r in rounds:
        lines.append(f"- Round {r['round']}: {r['hypothesis']} → **{r['verdict']}** ({r['kind']}). {r['reason']}"
                     + (f" Commit {r['commit']}." if r.get("commit") else ""))
    with open(os.path.join(out_dir, "report.md"), "w") as fh:
        fh.write("\n".join(lines) + "\n")
    return advice


# ---------------------------------------------------------------- the loop

def load_baseline(baseline_dir, tasks):
    """The baseline's with-skill runs, each tagged with its split."""
    split = {t["id"]: t["split"] for t in tasks}
    with open(os.path.join(baseline_dir, "graded.json")) as f:
        graded = [g for g in json.load(f) if g.get("arm") == "with"]
    for g in graded:
        g["split"] = split.get(g["task"])
    return [g for g in graded if g["split"]]


def per_round(tasks, runs):
    train = sum(1 for t in tasks if t["split"] == "tasks") * runs
    held = sum(1 for t in tasks if t["split"] == "heldout") * runs
    return train, held, 1 + train + held


def hillclimb(a, *, build=None, check=None, repo=None, tasks_root=None, out=None):
    repo = repo or suite.ROOT
    tasks = load_tasks(tasks_root or HERE)
    train_tasks = [t for t in tasks if t["split"] == "tasks"]
    build = build or suite.build
    check = check or check_skill
    n_train, n_held, round_cost = per_round(tasks, a.runs)
    baseline_runs = load_baseline(a.baseline, tasks)
    noise = noise_floor(baseline_runs, a.min_noise)
    base = {s: metrics(baseline_runs, s) for s in ("tasks", "heldout")}
    budget = Budget(a.max_sessions, a.stop_above)

    print(f"Per round: 1 proposer + {n_train} train + {n_held} held-out runs = {round_cost} sessions, "
          f"with the skill only. {a.rounds} rounds: at most {a.rounds * round_cost}; cap {a.max_sessions}.")
    def f3(x):
        return "n/a" if x is None else f"{x:.3f}"
    print(f"Noise floors from the baseline: train {f3(noise['tasks']['score'])}, "
          f"held out {f3(noise['heldout']['score'])}; baseline scores train {f3(base['tasks']['score'])}, "
          f"held out {f3(base['heldout']['score'])} ({len(baseline_runs)} with-skill runs).")
    dirty = git(repo, "status", "--porcelain", "--untracked-files=no")
    branch = git(repo, "branch", "--show-current")
    missing = sorted({t["id"] for t in tasks} - {g["task"] for g in baseline_runs})
    if a.dry_run:
        ok = not dirty and branch not in ("main", "master")
        print(f"[{'ok' if not dirty else 'FAIL'}] clean tree")
        print(f"[{'ok' if branch not in ('main', 'master') else 'FAIL'}] on a branch of its own: {branch}")
        print(f"[{'ok' if not missing else 'FAIL'}] baseline has with-skill runs of every task"
              + (f": missing {missing}" if missing else ""))
        ok = ok and not missing
        auth = subprocess.run([a.claude, "auth", "status", "--json"], capture_output=True, text=True)
        try:
            logged = json.loads(auth.stdout).get("loggedIn")
        except json.JSONDecodeError:
            logged = False
        print(f"[{'ok' if logged else 'FAIL'}] claude is logged in")
        ok = ok and bool(logged)
        print("Dry run: no session started." + ("" if ok else " Some checks FAILED."))
        return 0 if ok else 1
    if dirty:
        sys.exit("The tree is not clean: commit or stash first.")
    if branch in ("main", "master"):
        sys.exit("Run the hillclimb on a branch of its own: it commits there.")
    if missing:
        # Without them a split's baseline score is None, and the first
        # round would fail in decide() after spending its sessions.
        sys.exit(f"The baseline has no with-skill runs of {missing}: run it with --split all.")

    stamp = datetime.datetime.now().strftime("%Y%m%dT%H%M%S")
    out = out or os.path.join(HERE, "results", "hillclimb-" + stamp)
    os.makedirs(out, exist_ok=True)
    incumbent_runs = baseline_runs
    inc = base
    rounds = []
    idle = 0
    stopped = f"after {a.rounds} rounds"
    for n in range(1, a.rounds + 1):
        if not budget.affords(round_cost):
            stopped = budget.stopped or f"round {n} would pass the cap of {a.max_sessions} sessions"
            break
        rdir = os.path.join(out, f"round-{n}")
        os.makedirs(rdir)
        prompt = proposer_input(repo, failure_summary(incumbent_runs, train_tasks), rounds, a.candidates)
        with open(os.path.join(rdir, "proposer-input.md"), "w") as f:
            f.write(prompt)
        record = {"round": n, "hypothesis": "(none)", "verdict": "revert", "kind": "invalid", "reason": ""}
        try:
            try:
                hypothesis, diff = propose(prompt, claude=a.claude, model=a.proposer_model,
                                           log_path=os.path.join(rdir, "proposer.json"))
            finally:
                budget.spend()
            record["hypothesis"] = hypothesis
            with open(os.path.join(rdir, "patch.diff"), "w") as f:
                f.write(diff)
            apply_diff(repo, diff)
            check(repo)
        except ValueError as e:
            revert(repo)
            record["reason"] = str(e)
            rounds.append(record)
            idle += 1
            print(f"round {n}: invalid proposal: {e}")
            if idle >= a.patience:
                stopped = f"plateau: {idle} rounds in a row kept nothing"
                break
            continue
        try:
            binary = os.path.join(rdir, "cases")
            build(binary)
            plugin = os.path.join(rdir, "plugin")
            suite.make_plugin(binary, plugin)
            runs_dir = os.path.join(rdir, "runs")
            os.makedirs(runs_dir)
            cand_runs, complete = evaluate(tasks, binary=binary, plugin=plugin, claude=a.claude, model=a.model,
                                           effort=a.effort, runs=a.runs, concurrency=a.concurrency,
                                           out_dir=runs_dir, budget=budget)
        except BaseException:
            revert(repo)  # leave the tree clean, so a rerun can start
            raise
        if not complete:
            revert(repo)
            record.update(kind="stopped", reason=budget.stopped or "not every run finished")
            rounds.append(record)
            stopped = budget.stopped or "a round did not finish"
            break
        cand = {s: metrics(cand_runs, s) for s in ("tasks", "heldout")}
        verdict, kind, reason = decide(inc, cand, noise)
        train_note = f"training score {inc['tasks']['score'] or 0:.3f} -> {cand['tasks']['score'] or 0:.3f}"
        if kind == "regression" and "on train tasks" in reason:
            train_note += "; " + reason
        record.update(verdict=verdict, kind=kind, reason=reason, train=train_note)
        if verdict == "keep":
            record["commit"] = commit(repo, n, hypothesis, reason, inc, cand)
            inc, incumbent_runs = cand, cand_runs
            idle = 0
        else:
            revert(repo)
            idle += 1
        rounds.append(record)
        print(f"round {n}: {verdict} ({kind}): {reason}")
        with open(os.path.join(out, "rounds.json"), "w") as f:
            json.dump(rounds, f, indent=1)
        if idle >= a.patience:
            stopped = f"plateau: {idle} rounds in a row kept nothing"
            break
    with open(os.path.join(out, "rounds.json"), "w") as f:
        json.dump(rounds, f, indent=1)
    if stopped.startswith("plateau"):
        # Each training task's outcome pass rate in the baseline, so an outcome
        # that failed in every baseline run reads as an ambiguous task.
        outcomes = {}
        for g in baseline_runs:
            v = g["rules"].get("outcome", {}).get("result")
            if g["split"] == "tasks" and not g.get("infra_error") and v in ("pass", "fail"):
                outcomes.setdefault(g["task"], []).append(v == "pass")
        base_outcome = {t: mean(v) for t, v in outcomes.items()}
        with open(os.path.join(out, "root-causes.md"), "w") as f:
            f.write(root_causes(incumbent_runs, base_outcome))
    advice = final_report(out, base, inc, rounds, noise, stopped, budget)
    print(f"{advice} Report: {os.path.join(out, 'report.md')}")
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--baseline", required=True, help="the baseline's results directory (graded.json)")
    ap.add_argument("--rounds", type=int, default=5)
    ap.add_argument("--patience", type=int, default=2, help="stop after this many rounds in a row with nothing kept")
    ap.add_argument("--runs", type=int, default=3)
    ap.add_argument("--model", default="claude-sonnet-5-5")
    ap.add_argument("--effort", default="low")
    ap.add_argument("--proposer-model", default="claude-opus-5-5")
    ap.add_argument("-j", "--concurrency", type=int, default=4)
    ap.add_argument("--max-sessions", type=int, default=550)
    ap.add_argument("--stop-above", type=float, default=0.90,
                    help="start no new session once the account reports utilisation above this")
    ap.add_argument("--min-noise", type=float, default=0.0, help="a floor under every noise floor")
    ap.add_argument("--candidates", default=os.path.join(HERE, "hillclimb-candidates.md"))
    ap.add_argument("--claude", default=shutil.which("claude") or "claude")
    ap.add_argument("--dry-run", action="store_true", help="check and count, start no session")
    a = ap.parse_args(argv)
    return hillclimb(a)


if __name__ == "__main__":
    sys.exit(main())
