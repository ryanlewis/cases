"""Reads real Claude Code session transcripts (~/.claude/projects/*/*.jsonl)
into the harness's Transcript, so the eval's graders can run on history.

Real transcripts differ from the eval's stream-json: a background task's end
arrives as `<task-notification>` text (in a queued command, an attachment or
a user message) rather than a system event, subagent turns are marked
isSidechain, and the background task's output file is usually gone by the
time anyone reads the transcript. This module only reads.
"""
import hashlib
import json
import os
import re
import sys
import tempfile

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "harness"))
from transcript import Transcript, cases_segments, strip_heredocs  # noqa: E402

NOTE = re.compile(r"<task-notification>\s*<task-id>(\w+)</task-id>.*?<status>(\w+)</status>", re.S)
QUOTED = re.compile(r'"(?:[^"\\]|\\.)*"|\'[^\']*\'')
TURN_TEXT = ("<task-notification>", "<local-command", "<command-name>", "[SYSTEM NOTIFICATION")
# A quoted string that only names a shell variable ("$id") is a different value
# in each call, so it is kept as it is, not made a token of its text.
VAR_ONLY = re.compile(r'"\$\{?\w+\}?"')
# Meta user messages (a skill's text, idle notices) are not a human turn; a
# relayed message from another session is.
RELAYED = ("<cross-session-message", "<agent-message")


def real_segments(tool):
    """A tool's cases segments that act on the real store: calls that name
    another store (`--store`, `CASES_STORE=`) are CLI testing, not use."""
    if re.search(r"\bCASES_STORE=", tool.cmd):
        return []
    return [(s, a) for s, a in tool.segments if "--store" not in a]


def _token(m):
    q = m.group(0)
    if VAR_ONLY.fullmatch(q):
        return q
    return q[0] + "q" + hashlib.sha1(q.encode()).hexdigest()[:8] + q[0]


class Session:
    """One top-level session: its transcript, when it started, and the turn each tool call fell in."""

    def __init__(self, path):
        self.path = path
        self.id = os.path.basename(path)[:-len(".jsonl")]
        self.project = os.path.basename(os.path.dirname(path))
        events = []
        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                try:
                    d = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if d.get("isSidechain"):
                    continue
                events.append(d)
        self.events = events
        stamps = [d["timestamp"] for d in events if d.get("timestamp")]
        self.start = min(stamps) if stamps else ""
        self.end = max(stamps) if stamps else ""
        # The harness's Transcript reads a file of the conversation's events.
        with tempfile.NamedTemporaryFile("w", suffix=".jsonl", delete=False, encoding="utf-8") as tmp:
            for d in events:
                if d.get("type") in ("assistant", "user"):
                    tmp.write(json.dumps(d) + "\n")
        try:
            self.t = Transcript(tmp.name)
        finally:
            os.unlink(tmp.name)
        self._strip_heredocs()
        self._notifications()
        self._turns()

    def _strip_heredocs(self):
        """A heredoc's body is data (a script, a file, a case body), not commands:
        a Python edit that mentions `cases close` or `gh pr merge` runs neither.
        The cases calls are then read with quoted strings blanked too, since a
        `grep "cases park"` or a `sed` over the skill's text runs no cases call."""
        for tool in self.t.tools:
            if tool.name != "Bash":
                continue
            if "<<" in tool.cmd:
                tool.raw_cmd = tool.cmd
                tool.cmd = strip_heredocs(tool.cmd)
            # Each quoted string becomes a token of its content, so two values stay two.
            unquoted = QUOTED.sub(_token, tool.cmd)
            tool.segments = cases_segments(unquoted)
            tool.subs = [s for s, _ in tool.segments]

    def _notifications(self):
        """Marks each background task ended where its notification arrived, and
        takes its output from the file if it is still there, else from the
        first later call that read that file."""
        count = 0
        seen = set()
        for d in self.events:
            if d.get("type") == "assistant":
                count += sum(1 for b in d["message"].get("content", []) if b.get("type") == "tool_use")
                continue
            s = json.dumps(d)
            if "<task-notification>" not in s:
                continue
            for tid, status in NOTE.findall(s.replace("\\n", "\n")):
                tool = self.t.tasks.get(tid)
                if tool is None or tid in seen:
                    continue
                seen.add(tid)
                self.t.end_task(tool, status, count)
        for tid, tool in self.t.tasks.items():
            m = re.search(r"Output is being written to: (\S+?\.output)", tool.result or "")
            if m and os.path.exists(m.group(1)):
                with open(m.group(1), encoding="utf-8", errors="replace") as f:
                    tool.task_output = f.read()
                continue
            for later in self.t.tools[tool.index + 1:]:
                if tid + ".output" in json.dumps(later.input) and later.result:
                    tool.task_output = later.result
                    break

    def _turns(self):
        """Numbers each tool call by the human turn it fell in. A turn starts
        at a user message that is typed text, not a tool result or a
        notification."""
        turn = 0
        count = 0
        self.turn_of = []
        self.incoming = []   # (tool count, text) of each typed or relayed message
        for d in self.events:
            if d.get("type") == "user":
                c = d["message"].get("content")
                texts = [c] if isinstance(c, str) else [b.get("text", "") for b in c or []
                                                        if isinstance(b, dict) and b.get("type") == "text"]
                texts = [x for x in texts if x and not x.lstrip().startswith(TURN_TEXT)
                         and (not d.get("isMeta") or any(r in x for r in RELAYED))]
                if texts:
                    turn += 1
                    self.incoming.append((count, "\n".join(texts)))
            elif d.get("type") == "assistant":
                for b in d["message"].get("content", []):
                    if b.get("type") == "tool_use":
                        self.turn_of.append(turn)
                        count += 1
