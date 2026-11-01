#!/usr/bin/env bash
# Runs the full baseline of the cases skill eval, writes BASELINE.md, commits
# it and opens its pull request.
#
#   evals/cases-skill/run-baseline.sh                 run it
#   evals/cases-skill/run-baseline.sh --dry-run       check everything, start no session
#   evals/cases-skill/run-baseline.sh --resume DIR    finish a run that stopped halfway
set -euo pipefail

# Start no new session once the account reports more than this share of a
# rate limit used (Claude Code's rate_limit_event, any window). Sessions
# already running finish; --resume carries on after the limit resets.
STOP_ABOVE=0.90

AGENT_SESSIONS=204   # 34 tasks x 2 arms (with and without the skill) x 3 runs
JUDGE_SESSIONS=13    # 11 batches of up to 20 runs, then 2 to re-judge 40 runs for consistency
RUN_ARGS=(--split all --runs 3 --model claude-sonnet-5-5 --effort low -j 4 --stop-above "$STOP_ABOVE")

cd "$(dirname "$0")/../.."
dry=0
resume=""
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) dry=1 ;;
    --resume) resume="${2:?--resume needs the results directory}"; shift ;;
    *) echo "usage: $0 [--dry-run | --resume DIR]" >&2; exit 2 ;;
  esac
  shift
done

# SSH to GitHub fails now and then on this machine, so git's network calls get three tries.
retry() {
  local n
  for n in 1 2 3; do
    "$@" && return 0
    echo "retrying ($n): $*" >&2
    sleep 5
  done
  return 1
}

# Untracked files, such as RUN.md beside the checkout, do not change the build.
if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
  echo "The tree is not clean. Commit or stash first." >&2
  exit 1
fi
retry git fetch -q origin
if ! git merge-base --is-ancestor origin/main HEAD; then
  echo "main has moved: rebasing $(git branch --show-current) onto origin/main"
  git rebase -q origin/main
fi
# run.py builds the binary under test again into the results directory; this
# only fails early if it does not build.
cargo build --release --locked -p cases

echo "Sessions: $AGENT_SESSIONS agent + $JUDGE_SESSIONS judge = $((AGENT_SESSIONS + JUDGE_SESSIONS)) claude -p sessions."
echo "Stops starting sessions if the account reports utilisation above $STOP_ABOVE."

if [ "$dry" = 1 ]; then
  exec python3 evals/cases-skill/run.py "${RUN_ARGS[@]}" --dry-run
fi

out="${resume:-evals/cases-skill/results/baseline-$(date +%Y%m%dT%H%M%S)}"
if [ ! -f "$out/graded.json" ]; then
  # Exits non-zero, having graded nothing, if runs are left unfinished.
  python3 evals/cases-skill/run.py "${RUN_ARGS[@]}" --out "$out" ${resume:+--resume}
fi
# Consistency: grade the same outputs again. Programmatic verdicts must not
# change; 40 runs are judged again and the summary gives the agreement. The
# marker lets --resume redo it when the machine stopped during it.
if [ ! -f "$out/.regraded" ]; then
  python3 evals/cases-skill/run.py grade "$out" --judge-limit 40
  touch "$out/.regraded"
fi
python3 evals/cases-skill/run.py baseline "$out"

git add evals/cases-skill/BASELINE.md
git commit -q -m "test(skill): baseline for the cases skill eval"
retry git push -q --force-with-lease -u origin HEAD
body="$(mktemp)"
cat > "$body" <<BODY
Adds BASELINE.md: the first measured run of the cases skill eval, before any change to the skill, for a hillclimb to compare against.

It ran every task, train and held out, 3 times with the skill and 3 times without, on claude-sonnet-5-5 at low effort, with the haiku judge. BASELINE.md gives each rule's pass rate with a 95% Wilson interval, cost per decision, run-to-run variance, headroom, the judge's agreement on re-grading, and infra failures apart from task failures.

Raw results stay in $out, which is gitignored.
BODY
gh pr create --base main --title "test(skill): baseline for the cases skill eval" --body-file "$body"
