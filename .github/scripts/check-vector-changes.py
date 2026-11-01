#!/usr/bin/env python3
"""Refuse a vector change that spec/vectors/CHANGES.md does not log.

The vectors are the core's contract, so a change to them has to be one a
reviewer can see and read about. Against the merge base of BASE and HEAD:

- when any spec/vectors/*.json or spec/vectors/store/*.json file changed,
  CHANGES.md must change too;
- each vector added, removed or changed, named by its description, must
  appear in backticks on a line this diff adds to CHANGES.md.

Usage: check-vector-changes.py BASE
BASE is a commit or ref, such as origin/main. Only the standard library is
used, so it runs on any runner with Python 3.
"""

import json
import subprocess
import sys

VECTORS = "spec/vectors/"
CHANGES = "spec/vectors/CHANGES.md"


def git(*args):
    return subprocess.run(
        ["git", *args], check=True, capture_output=True, text=True
    ).stdout


def vectors_at(rev, path):
    """The file's vectors at rev, by description; empty when it has none."""
    try:
        text = git("show", f"{rev}:{path}")
    except subprocess.CalledProcessError:
        return {}
    try:
        return {v["description"]: v for v in json.loads(text)}
    except (ValueError, TypeError, KeyError) as e:
        sys.exit(f"{path} at {rev} does not read as a list of vectors: {e}")


def canonical(vector):
    return json.dumps(vector, sort_keys=True, separators=(",", ":"))


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    base = git("merge-base", sys.argv[1], "HEAD").strip()
    changed = git("diff", "--name-only", "--no-renames", base, "HEAD", "--", VECTORS).split()
    # The case vectors, and the store vectors in store/.
    vector_files = [
        p
        for p in changed
        if p.endswith(".json")
        and "/" not in p[len(VECTORS) :].removeprefix("store/")
    ]
    if not vector_files:
        print("No vector file changed.")
        return 0

    problems = []
    if CHANGES not in changed:
        problems.append(
            f"{', '.join(vector_files)} changed, but {CHANGES} did not."
        )

    diff = git("diff", "--unified=0", base, "HEAD", "--", CHANGES)
    logged = "\n".join(
        line[1:]
        for line in diff.splitlines()
        if line.startswith("+") and not line.startswith("+++")
    )

    for path in vector_files:
        before = vectors_at(base, path)
        after = vectors_at("HEAD", path)
        edits = [("added", d) for d in after if d not in before]
        edits += [("removed", d) for d in before if d not in after]
        edits += [
            ("changed", d)
            for d in after
            if d in before and canonical(after[d]) != canonical(before[d])
        ]
        for what, description in edits:
            if f"`{description}`" not in logged:
                problems.append(
                    f"{path}: `{description}` was {what}, but no line added "
                    f"to {CHANGES} names it."
                )

    if problems:
        print("Vector changes must be logged in CHANGES.md:\n", file=sys.stderr)
        for p in problems:
            print(f"- {p}", file=sys.stderr)
        print(
            f"\nAdd a line to {CHANGES} for each, naming the vector by its "
            "description in backticks, with the story or decision behind it.",
            file=sys.stderr,
        )
        return 1
    print(f"Every vector change in {', '.join(vector_files)} is logged.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
