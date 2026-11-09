"""Reads a `claude -p --output-format stream-json` transcript.

It yields the tool calls in order, with their results and background tasks,
the API calls with their usage, and the `cases` subcommands in each Bash
command. The parsing of `cases` commands and the cost of an API call follow
the cost study's script (notes: projects/cases/research/cases-cost-in-sessions.py).
"""
import json
import re
import shlex

# The subcommands `cases` has. A segment counts as a `cases` call only when a
# real subcommand follows, so prose in a heredoc is not matched.
SUBS = set(
    "amend answer close config doing inbox list message note open park pickup prune resume "
    "serve service show skill status sweep wait withdraw listeners".split()
)
# A command segment starts at the start, after &&, ||, ;, |, a newline, ( or $(.
SEG = re.compile(
    r'(?:^|&&|\|\||[;|\n(]|\$\()\s*(?:[A-Z_]+=\S+\s+)*((?:\S*/)?cases)\s+'
    r'((?:--?\S+(?:\s+(?!(?:%s)\b)\S+)?\s+)*)(\w+)' % "|".join(SUBS)
)
# Where a segment ends, to cut out its arguments.
END = re.compile(r'&&|\|\||;|\n(?!\s)|\|(?!\|)')

# A heredoc's body is data, such as a case body written to a file, not
# commands. The rest of its opening line is kept: `<<EOF && git push` runs git.
HEREDOC = re.compile(r"<<-?\s*(['\"]?)(\w+)\1([^\n]*)\n.*?\n\s*\2(?=\s|$)", re.S)


def strip_heredocs(cmd):
    """A command with each heredoc's body taken out, as `<<HEREDOC`."""
    return HEREDOC.sub(lambda m: "<<HEREDOC" + m.group(3), cmd or "")


# What may stand beside `cases` calls in a command that is still bookkeeping.
# Control words are skipped, so the command after them is what counts.
CONTROL = set("if then elif else do while until ! { } ( ) time".split())
# Glue that moves or tests text: it does no work whatever its arguments,
# except that a redirection it makes must go to a file the case uses.
GLUE = set("echo printf cd true false test [ [[ ] ]] fi done : exit".split())
# Text tools: glue only when every file they read or write is one the case
# uses (a body file, a wait's output, stdin), so `cat src/main.rs` is work.
# The number is how many operands come before the files (a pattern, a program).
TEXT = {"cat": 0, "head": 0, "tail": 0, "wc": 0, "sort": 0, "uniq": 0, "cut": 0,
        "grep": 1, "sed": 1, "awk": 1, "jq": 1, "tr": 2}
CASES = re.compile(r"(?:\S*/)?cases")
BODY_FILE = re.compile(r"--(?:body|outcome|text)-file[=\s]+(\"[^\"]*\"|'[^']*'|\S+)")
REDIRECT = re.compile(r"^\d*(?:>>?|&>|<)(.*)$")

EXIT = re.compile(r'Exit code (\d+)')
BG = re.compile(r'running in background with ID: (\w+)')


def _segment_end(rest):
    """Where a segment's arguments end: the first separator outside quotes, so
    a `;` or `|` inside a quoted title or outcome does not cut the arguments.
    With a quote left open, as an apostrophe in heredoc prose leaves one, the
    first separator anywhere."""
    quote = None
    i = 0
    while i < len(rest):
        ch = rest[i]
        if quote:
            if ch == "\\" and quote == '"':
                i += 2
                continue
            if ch == quote:
                quote = None
        elif ch == "\\":
            i += 2
            continue
        elif ch in "'\"":
            quote = ch
        elif END.match(rest, i):
            return i
        i += 1
    if quote:
        e = END.search(rest)
        return e.start() if e else len(rest)
    return len(rest)


def cases_segments(cmd):
    """The `cases` calls in a shell command, as (subcommand, argument text)."""
    out = []
    for m in SEG.finditer(cmd or ""):
        sub = m.group(3)
        if sub not in SUBS:
            continue
        rest = cmd[m.end():]
        # Arguments run to the end of the segment. A backslash-newline continues it.
        rest = rest.replace("\\\n", " ")
        args = (m.group(2) or "") + " " + rest[: _segment_end(rest)]
        # A call for help is reading, not using the command.
        if re.search(r'(?:^|\s)(?:--help|-h)(?:\s|$)', args):
            continue
        out.append((sub, args))
    return out


def _scan(cmd):
    """A command's simple commands, split at &&, ||, ;, |, &, a newline, ( and
    ), outside quotes and command substitutions, which nest as the shell nests
    them. Each `$(...)` in a piece is replaced by `$SUB`, and its body is
    returned beside the pieces, to be checked in its turn."""
    cmd = strip_heredocs(cmd).replace("\\\n", " ")
    pieces, subs, cur, stack, start, i = [], [], [], [], 0, 0
    while i < len(cmd):
        ch, top = cmd[i], (stack[-1] if stack else None)
        outer = "$(" not in stack and "(" not in stack
        if top == "'":
            if ch == "'":
                stack.pop()
        elif ch == "\\":
            if outer:
                cur.append(cmd[i:i + 2])
            i += 2
            continue
        elif top == '"' and ch == '"':
            stack.pop()
        elif cmd.startswith("$(", i):
            if outer:
                start = i + 2
                cur.append("$SUB")
            stack.append("$(")
            i += 2
            continue
        elif top == '"':
            pass
        elif ch in "'\"":
            stack.append(ch)
        elif top in ("$(", "(") and ch == "(":
            stack.append("(")
        elif top in ("$(", "(") and ch == ")":
            stack.pop()
            if not stack or ("$(" not in stack and "(" not in stack):
                subs.append(cmd[start:i])
                i += 1
                continue
        elif top is None and (cmd.startswith("&&", i) or cmd.startswith("||", i)):
            pieces.append("".join(cur))
            cur = []
            i += 2
            continue
        elif top is None and (ch in ";|\n()" or (ch == "&" and cmd[i - 1:i] not in (">", "<")
                                                  and cmd[i + 1:i + 2] != ">")):
            pieces.append("".join(cur))
            cur = []
            i += 1
            continue
        if outer:
            cur.append(ch)
        i += 1
    pieces.append("".join(cur))
    return [p.strip() for p in pieces if p.strip()], subs


def _words(piece):
    try:
        return shlex.split(piece, comments=True)
    except ValueError:
        return piece.split()


def _files_ok(words, allowed):
    """Whether every file a text tool reads or writes, after its leading
    operands, is one the case uses. Flags and numbers are not files."""
    skip = TEXT.get(words[0], 0)
    for w in words[1:]:
        r = REDIRECT.match(w)
        if r:
            continue
        if w.startswith("-") and w != "-":
            continue
        if skip:
            skip -= 1
            continue
        if w.isdigit() or w.startswith("<<"):
            continue
        if not allowed(w):
            return False
    return True


def _redirects_ok(words, allowed):
    """Whether every file a command redirects to or from is one the case uses."""
    for k, w in enumerate(words):
        r = REDIRECT.match(w)
        if not r or w.startswith("<<"):
            continue
        target = r.group(1) or (words[k + 1] if k + 1 < len(words) else "")
        if target.startswith("&") or target == "":
            continue
        if not allowed(target):
            return False
    return True


def _bookkeeping(piece, allowed):
    """Whether one simple command is cases bookkeeping: a `cases` call, an
    assignment, a comment, glue, or a text tool on the case's files."""
    words = _words(piece)
    while words and (words[0] in CONTROL or re.match(r"^[A-Za-z_]\w*=", words[0])):
        words = words[1:]
    if not words or words[0] == "for":
        return True
    head = words[0]
    if CASES.fullmatch(head):
        return True
    if head in GLUE:
        return _redirects_ok(words, allowed)
    if head in TEXT:
        return _files_ok(words, allowed) and _redirects_ok(words, allowed)
    return False


def bookkeeping_only(cmd, wait_tasks=()):
    """Whether every simple command of a shell command, those inside command
    substitutions included, is cases bookkeeping. A file counts as the case's
    when it is a `--body-file`, `--outcome-file` or `--text-file` of a `cases`
    call in the command, the output of one of `wait_tasks` (background task
    ids of `cases wait`), stdin or /dev/null."""
    named = {f.strip("\"'") for f in BODY_FILE.findall(strip_heredocs(cmd))}

    def allowed(path):
        return (path in named or path in ("-", "/dev/null", "/dev/stdout", "/dev/stderr")
                or any(tid + ".output" in path for tid in wait_tasks))

    def check(text, depth=0):
        pieces, subs = _scan(text)
        return (all(_bookkeeping(p, allowed) for p in pieces)
                and (depth > 8 or all(check(s, depth + 1) for s in subs)))
    return check(cmd)


def has_flag(args, flag):
    return re.search(r'(?:^|\s)%s(?:[=\s]|$)' % re.escape(flag), args) is not None


def flag_value(args, flag):
    m = re.search(r'(?:^|\s)%s[=\s]+("([^"]*)"|\'([^\']*)\'|(\S+))' % re.escape(flag), args)
    if not m:
        return None
    return next(g for g in m.groups()[1:] if g is not None)


def first_positional(args):
    """The first word of a segment's arguments that is not a flag or a flag's value."""
    words = re.findall(r'"[^"]*"|\'[^\']*\'|\S+', args)
    skip = False
    for w in words:
        if skip:
            skip = False
            continue
        if w.startswith("-"):
            # Boolean flags we know take no value.
            if w in ("--answer", "--json", "-j", "--pickup", "--all", "--count", "--accept",
                     "--changes", "--drop", "--ack", "--other"):
                continue
            if "=" not in w:
                skip = True
            continue
        return w.strip("\"'")
    return None


class Tool:
    def __init__(self, index, call_index, block):
        self.index = index            # order among tool calls
        self.call_index = call_index  # which API call made it
        self.id = block["id"]
        self.name = block["name"]
        self.input = block.get("input") or {}
        self.cmd = self.input.get("command", "") if self.name == "Bash" else ""
        self.bg = bool(self.input.get("run_in_background"))
        self.segments = cases_segments(self.cmd)
        self.subs = [s for s, _ in self.segments]
        self.result = None
        self.is_error = False
        self.exit = None
        self.task_id = None           # when run in the background
        self.task_output = None       # the background task's output, read after the run
        self.task_status = None
        self.done_at = None           # tools made before a background task ended

    def text(self):
        """What the command printed, in the foreground or the background."""
        if self.bg:
            return self.task_output or ""
        return self.result or ""


class Transcript:
    def __init__(self, path):
        self.events = []
        with open(path, encoding="utf-8") as f:
            lines = f.readlines()
        for line in lines:
            line = line.strip()
            if not line:
                continue
            try:
                self.events.append(json.loads(line))
            except json.JSONDecodeError:
                self.events.append({"type": "raw", "text": line})
        self.calls = {}      # message id -> usage
        self.call_order = []
        self.tools = []
        self.by_id = {}
        self.tasks = {}      # task id -> Tool
        self.results = []    # result events
        self.texts = []      # assistant text blocks
        self.skill_invoked = []
        self.errors = []     # API or process errors
        for d in self.events:
            t = d.get("type")
            if t == "assistant":
                m = d["message"]
                mid = m.get("id") or d.get("uuid")
                if mid not in self.calls:
                    self.call_order.append(mid)
                self.calls[mid] = m.get("usage") or self.calls.get(mid) or {}
                for b in m.get("content", []):
                    if b.get("type") == "tool_use":
                        tool = Tool(len(self.tools), len(self.call_order) - 1, b)
                        self.tools.append(tool)
                        self.by_id[tool.id] = tool
                        if tool.name == "Skill":
                            self.skill_invoked.append(tool.input.get("skill"))
                    elif b.get("type") == "text":
                        self.texts.append(b.get("text", ""))
            elif t == "user":
                content = d["message"].get("content")
                if isinstance(content, list):
                    for b in content:
                        if b.get("type") != "tool_result":
                            continue
                        tool = self.by_id.get(b.get("tool_use_id"))
                        if tool is None:
                            continue
                        c = b.get("content")
                        s = c if isinstance(c, str) else "\n".join(
                            x.get("text", "") for x in (c or []) if isinstance(x, dict))
                        tool.result = s
                        tool.is_error = bool(b.get("is_error"))
                        m = EXIT.search(s) if tool.is_error else None
                        tool.exit = int(m.group(1)) if m else (1 if tool.is_error else 0)
                        mb = BG.search(s)
                        if mb:
                            tool.task_id = mb.group(1)
                            self.tasks[tool.task_id] = tool
            elif t == "system" and d.get("subtype") == "task_notification":
                tool = self.tasks.get(d.get("task_id")) or self.by_id.get(d.get("tool_use_id"))
                if tool is not None:
                    self.end_task(tool, d.get("status"), len(self.tools))
                    tool.task_output_file = d.get("output_file")
            elif t == "result":
                self.results.append(d)
                if d.get("is_error") or d.get("subtype", "success") != "success":
                    self.errors.append(d.get("subtype") or "error")
            elif t == "raw":
                if "API Error" in d["text"] or "Error:" in d["text"]:
                    self.errors.append(d["text"][:200])
        self._stops()

    STOPPERS = ("TaskStop", "KillShell", "KillBash")

    @staticmethod
    def end_task(tool, status, at):
        """Records that a background task ended before tool call `at`. The
        earliest end wins: a task the agent stopped ended at the stop, though
        its notification may come later, and the earliest end gives the status.
        A notification at the same point as a stop came before the stopping
        call, so it wins whichever is recorded first."""
        if (tool.done_at is None or at < tool.done_at
                or (at == tool.done_at and tool.task_status == "stopped" and status != "stopped")):
            tool.done_at = at
            tool.task_status = status

    def _stops(self):
        """A task the agent stopped ended at the stop, notified or not."""
        for stopper in self.tools:
            # A stop that failed (the task had already ended) stopped nothing.
            if stopper.name in self.STOPPERS and not stopper.is_error:
                tid = stopper.input.get("task_id") or stopper.input.get("shell_id") or stopper.input.get("id")
                tool = self.tasks.get(tid)
                if tool is not None:
                    self.end_task(tool, "stopped", stopper.index)

    def attach_task_outputs(self, outputs):
        """outputs: task id -> text of its output file, saved by the runner."""
        for tid, text in outputs.items():
            tool = self.tasks.get(tid)
            if tool is not None:
                tool.task_output = text

    def cases_tools(self):
        return [t for t in self.tools if t.segments]

    def usage_totals(self):
        tot = {"input": 0, "cache_read": 0, "cache_write": 0, "output": 0}
        for mid in self.call_order:
            u = self.calls[mid]
            tot["input"] += u.get("input_tokens", 0)
            tot["cache_read"] += u.get("cache_read_input_tokens", 0)
            tot["cache_write"] += u.get("cache_creation_input_tokens", 0)
            tot["output"] += u.get("output_tokens", 0)
        return tot

    def cost_usd(self):
        return sum(r.get("total_cost_usd") or 0 for r in self.results)

    def call_classes(self):
        """Sort each API call that made only `cases` bookkeeping into ask or extra.

        A call counts only when each of its tool calls is bookkeeping: a Bash
        command with a `cases` call or a read of a wait's output in it whose
        every simple command, inside `$(...)` too, is bookkeeping (see
        bookkeeping_only); another tool reading a wait's output; or the cases
        skill. Work chained with bookkeeping, such as `git commit ... && cases
        close ...`, is not counted: it is the call the skill asks for. A wait
        is a background task whose command ran `cases wait`.

        An ask opens a case (in chat, that is the turn that asks); every other
        cases-only call (close, a separate wait, reading wait output, show,
        pickup, the skill) is extra: chat would not need it. `calls` lists
        the API calls (by call index) counted as either.
        """
        per_call = {}
        for t in self.tools:
            per_call.setdefault(t.call_index, []).append(t)
        ask = extra = 0
        calls = []
        wait_files = {tid for tid, t in self.tasks.items() if "wait" in t.subs}
        for ci, ts in per_call.items():
            tagged = []
            for t in ts:
                reads = any(tid + ".output" in json.dumps(t.input) for tid in wait_files)
                if t.name == "Skill" and "cases" in str(t.input.get("skill", "")):
                    tagged.append({"skill"})
                elif t.name == "Bash" and (t.segments or reads) and bookkeeping_only(t.cmd, wait_files):
                    tagged.append(set(t.subs) | ({"wait-read"} if reads else set()))
                elif t.name != "Bash" and reads:
                    tagged.append({"wait-read"})
                else:
                    tagged.append(None)
            if not tagged or any(x is None for x in tagged):
                continue
            subs = set().union(*tagged)
            calls.append(ci)
            if "open" in subs:
                ask += 1
            else:
                extra += 1
        return {"ask": ask, "extra": extra, "calls": calls}
