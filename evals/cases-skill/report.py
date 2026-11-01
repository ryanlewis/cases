"""Writes summary.md and summary.json for a graded results directory."""
import collections
import json
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "harness"))
from stats import fmt_rate, mean, stdev, wilson  # noqa: E402

HEADROOM = 0.95


def _rate(gs, rule):
    vals = [g["rules"].get(rule, {}).get("result") for g in gs]
    vals = [v for v in vals if v in ("pass", "fail")]
    return sum(v == "pass" for v in vals), len(vals)


def run_score(g):
    vals = [v["result"] for v in g["rules"].values() if v["result"] in ("pass", "fail")]
    return sum(v == "pass" for v in vals) / len(vals) if vals else None


def write(out_dir, graded, meta, tasks):
    arms = meta.get("arms", ["with", "without"])
    scored = [g for g in graded if not g.get("infra_error") and g.get("cost")]
    infra = [g for g in graded if g.get("infra_error") or not g.get("cost")]
    by_arm = {a: [g for g in scored if g["arm"] == a] for a in arms}
    rules = []
    for g in graded:
        for k in g["rules"]:
            if k not in rules:
                rules.append(k)
    lines = [f"# cases skill eval: {meta.get('started')}", ""]
    lines.append(f"Model {meta.get('model')} at effort {meta.get('effort')}, split {meta.get('split')}, "
                 f"{len(meta.get('tasks', []))} tasks, {meta.get('runs')} runs per arm, commit {meta.get('commit', '')[:10]}.")
    lines.append(f"Sessions: {meta.get('sessions')} agent, {meta.get('judge_sessions', 0)} judge.")
    lines.append(f"Cases from the suite's worker names in the real store (read only): before "
                 f"{meta.get('real_store_before')}, after {meta.get('real_store_after')}.")
    lines.append("")
    lines.append("## Infra failures (not scored)")
    lines.append("")
    if infra:
        for g in infra:
            lines.append(f"- {g['run']}: {g.get('infra_error')}")
    else:
        lines.append("None.")
    timeouts = [g["run"] for g in scored if g.get("timed_out")]
    lines.append("")
    lines.append(f"Timed out (scored as the agent left it): {len(timeouts)} {', '.join(timeouts)}")
    lines.append("")
    lines.append("## Rules, pass rate with 95% Wilson interval")
    lines.append("")
    lines.append("| rule | " + " | ".join(arms) + " |")
    lines.append("| --- |" + " --- |" * len(arms))
    summary = {"rules": {}, "arms": {}, "infra": [g["run"] for g in infra]}
    for rule in rules:
        row = []
        summary["rules"][rule] = {}
        for a in arms:
            p, n = _rate(by_arm[a], rule)
            row.append(fmt_rate(p, n))
            summary["rules"][rule][a] = [p, n]
        lines.append(f"| {rule} | " + " | ".join(row) + " |")
    lines.append("")
    lines.append("## Overall")
    lines.append("")
    for a in arms:
        gs = by_arm[a]
        p = sum(_rate(gs, r)[0] for r in rules)
        n = sum(_rate(gs, r)[1] for r in rules)
        scores = [run_score(g) for g in gs]
        perfect = sum(1 for s in scores if s == 1.0)
        op, on = _rate(gs, "outcome")
        rate = wilson(p, n)[0]
        summary["arms"][a] = {"rule_checks": [p, n], "mean_run_score": mean(scores),
                              "perfect_runs": perfect, "runs": len(gs), "outcome": [op, on]}
        lines.append(f"- **{a}**: rule checks {fmt_rate(p, n)}; mean run score "
                     f"{100 * (mean(scores) or 0):.0f}% (sd {100 * stdev(scores):.0f}); "
                     f"perfect runs {perfect}/{len(gs)}; task outcome {fmt_rate(op, on)}")
        if a == "with" and rate is not None and rate > HEADROOM:
            lines.append(f"  - **Headroom warning**: above {int(HEADROOM * 100)}%, the suite is too easy.")
    lines.append("")
    lines.append("By split (the hillclimb trains on train and checks on held out):")
    lines.append("")
    for split in ("tasks", "heldout"):
        for a in arms:
            gs = [g for g in by_arm[a] if (tasks.get(g["task"]) or {}).get("split") == split]
            if not gs:
                continue
            p = sum(_rate(gs, r)[0] for r in rules)
            n = sum(_rate(gs, r)[1] for r in rules)
            name = "train" if split == "tasks" else "held out"
            summary["arms"][a][name] = [p, n]
            lines.append(f"- {name}, {a}: rule checks {fmt_rate(p, n)}; mean run score "
                         f"{100 * (mean([run_score(g) for g in gs]) or 0):.0f}%")
    lines.append("")
    lines.append("## Judge claims (LLM)")
    lines.append("")
    claims = sorted({c for g in scored for c in (g.get("judge") or {})})
    if claims:
        lines.append("| claim | " + " | ".join(arms) + " |")
        lines.append("| --- |" + " --- |" * len(arms))
        for c in claims:
            row = []
            for a in arms:
                vs = [(g.get("judge") or {}).get(c, {}).get("v") for g in by_arm[a]]
                vs = [v for v in vs if v in (True, False)]
                row.append(fmt_rate(sum(vs), len(vs)))
            lines.append(f"| {c} | " + " | ".join(row) + " |")
    else:
        lines.append("Not run.")
    if meta.get("judge_agreement") is not None:
        lines.append("")
        lines.append(f"Judge agreement with the previous grading: {meta['judge_agreement']}")
    lines.append("")
    lines.append("## Cost")
    lines.append("")
    lines.append("Per run, mean (sd); per decision is the run's total over the human's answers, parks, "
                 "resumes, withdrawals and messages. Extra calls are API calls that made only `cases` "
                 "calls other than an open, as in the cost study.")
    lines.append("")
    lines.append("| arm | API calls | extra calls | turns | context tokens | $ | API calls / decision | extra / decision | $ / decision |")
    lines.append("| --- | --- | --- | --- | --- | --- | --- | --- | --- |")
    for a in arms:
        cs = [g["cost"] for g in by_arm[a]]
        if not cs:
            continue

        def ms(key):
            xs = [c[key] for c in cs]
            return f"{mean(xs):.1f} ({stdev(xs):.1f})"

        def per(key):
            xs = [c[key] / c["decisions"] for c in cs if c["decisions"]]
            return f"{mean(xs):.2f}" if xs else "n/a"
        ctx = [c["context_tokens"] for c in cs]
        lines.append(f"| {a} | {ms('api_calls')} | {ms('extra_calls')} | {ms('turns')} | {mean(ctx):,.0f} | "
                     f"{ms('cost_usd')} | {per('api_calls')} | {per('extra_calls')} | {per('cost_usd')} |")
        summary["arms"].setdefault(a, {})["cost"] = {
            k: mean([c[k] for c in cs]) for k in ("api_calls", "extra_calls", "turns", "cost_usd", "decisions")}
    lines.append("")
    for a in arms:
        gs = by_arm[a]
        if not gs:
            continue
        inv = sum(1 for g in gs if g["cost"]["skill_invoked"])
        show = sum(1 for g in gs if g["cost"]["skill_show_run"])
        lines.append(f"- {a}: Skill tool used in {inv}/{len(gs)} runs; `cases skill show` run in {show}/{len(gs)}.")
    lines.append("")
    lines.append("## Run-to-run variance")
    lines.append("")
    lines.append("Tasks whose runs in one arm disagree on at least one rule, and the spread of run scores.")
    lines.append("")
    per_task = collections.defaultdict(list)
    for g in scored:
        per_task[(g["task"], g["arm"])].append(g)
    for a in arms:
        split = [k for k in per_task if k[1] == a and len(per_task[k]) > 1]
        disagree = []
        sds = []
        for k in split:
            gs = per_task[k]
            sds.append(stdev([run_score(g) for g in gs]))
            for rule in rules:
                vals = {g["rules"].get(rule, {}).get("result") for g in gs} - {"na", None}
                if len(vals) > 1:
                    disagree.append(k[0])
                    break
        lines.append(f"- {a}: {len(disagree)}/{len(split)} tasks disagree; mean sd of run score "
                     f"{100 * (mean(sds) or 0):.0f} points. {', '.join(sorted(set(disagree)))}")
    lines.append("")
    lines.append("## Per task (with / without, mean run score)")
    lines.append("")
    lines.append("| task | split | " + " | ".join(arms) + " | failing rules (with) |")
    lines.append("| --- | --- |" + " --- |" * len(arms) + " --- |")
    for tid in meta.get("tasks", []):
        row = []
        for a in arms:
            s = [run_score(g) for g in per_task.get((tid, a), [])]
            row.append(f"{100 * mean(s):.0f}%" if s and mean(s) is not None else "n/a")
        fails = collections.Counter(r for g in per_task.get((tid, "with"), []) for r, v in g["rules"].items()
                                    if v["result"] == "fail")
        split = (tasks.get(tid) or {}).get("split", "")
        lines.append(f"| {tid} | {split} | " + " | ".join(row) + " | "
                     + ", ".join(f"{r}×{n}" for r, n in fails.most_common()) + " |")
    lines.append("")
    if meta.get("regrade_diffs") is not None:
        lines.append(f"Programmatic re-grade: {len(meta['regrade_diffs'])} verdicts changed.")
    with open(os.path.join(out_dir, "summary.md"), "w") as f:
        f.write("\n".join(lines) + "\n")
    with open(os.path.join(out_dir, "summary.json"), "w") as f:
        json.dump(summary, f, indent=1)
