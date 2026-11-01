"""Runs one task: a fresh workspace and case store, a Claude Code session, and
a scripted human who answers from outside the session.

The session is `claude -p` with streaming input, kept open while the agent has
a background command running, so a background `cases wait` that returns wakes
the agent as it would in an interactive session. Plain `claude -p` ends the
session when the agent ends its turn and kills the wait, which is why
`claude plugin eval` cannot host this suite (see the suite's README).

Isolation: the store is a file in the run's workspace, named by CASES_STORE and
passed as --store to every command the human runs; the XDG directories point
into the workspace too. The agent's Bash runs in Claude Code's OS sandbox,
which confines writes to the workspace, so no command it runs can write the
real store. Only project and local settings load, so the user's own skills,
CLAUDE.md and memory do not.
"""
import json
import os
import re
import shutil
import signal
import subprocess
import tempfile
import threading
import time

from transcript import cases_segments

HUMAN = "sam"
POLL = 0.3


def sub_vars(value, env):
    if isinstance(value, list):
        return [sub_vars(v, env) for v in value]
    return re.sub(r"\{(\w+)\}", lambda m: env.get(m.group(1), m.group(0)), value)


class Store:
    """The run's case store, read and written with the binary under test."""

    def __init__(self, binary, path, env):
        self.binary = binary
        self.path = path
        self.env = env

    def run(self, args, check=False):
        p = subprocess.run([self.binary, *args, "--store", self.path], env=self.env,
                           capture_output=True, text=True, timeout=30)
        if check and p.returncode != 0:
            raise RuntimeError("cases %s: exit %d: %s" % (" ".join(args), p.returncode, p.stderr.strip()))
        return p

    def cases(self):
        p = self.run(["list", "--all", "--json"])
        if p.returncode != 0:
            return []
        return json.loads(p.stdout).get("list", [])


class Session:
    """A Claude Code session fed through streaming input."""

    def __init__(self, argv, env, cwd, transcript_path):
        self.out = open(transcript_path, "w", encoding="utf-8")
        self.proc = subprocess.Popen(argv, env=env, cwd=cwd, stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                     text=True, start_new_session=True)
        self.lock = threading.Lock()
        self.last_event = time.time()
        self.last_type = None
        self.results = 0
        self.running = {}         # background task id -> command
        self.fg_waits = set()     # tool ids of foreground `cases wait` calls
        self.waits_started = 0
        self.output_files = {}    # task id -> path of its output file
        self.pending_bg = {}      # tool id -> command, until its task id is known
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()

    def send(self, text):
        msg = {"type": "user", "message": {"role": "user", "content": text}}
        self.proc.stdin.write(json.dumps(msg) + "\n")
        self.proc.stdin.flush()

    def waiting(self):
        """Whether the agent has a `cases wait` running, in the background or not."""
        with self.lock:
            return bool(self.fg_waits) or any(
                any(s == "wait" for s, _ in cases_segments(c)) for c in self.running.values())

    def _read(self):
        for line in self.proc.stdout:
            self.out.write(line)
            self.out.flush()
            try:
                d = json.loads(line)
            except json.JSONDecodeError:
                continue
            with self.lock:
                self.last_event = time.time()
                t = d.get("type")
                # System events (task changes, rate limits) do not end a turn
                # or start one, so idle() looks only at the conversation.
                if t in ("assistant", "user", "result"):
                    self.last_type = t
                if t == "result":
                    self.results += 1
                elif t == "assistant":
                    for b in d["message"].get("content", []):
                        if b.get("type") != "tool_use" or b.get("name") != "Bash":
                            continue
                        cmd = b["input"].get("command", "")
                        is_wait = any(s == "wait" for s, _ in cases_segments(cmd))
                        if b["input"].get("run_in_background"):
                            self.pending_bg[b["id"]] = cmd
                        elif is_wait:
                            self.fg_waits.add(b["id"])
                        if is_wait:
                            self.waits_started += 1
                elif t == "user":
                    content = d["message"].get("content")
                    for b in content if isinstance(content, list) else []:
                        if b.get("type") != "tool_result":
                            continue
                        self.fg_waits.discard(b.get("tool_use_id"))
                        c = b.get("content")
                        s = c if isinstance(c, str) else json.dumps(c)
                        m = re.search(r"running in background with ID: (\w+)", s)
                        if m and b.get("tool_use_id") in self.pending_bg:
                            self.running[m.group(1)] = self.pending_bg.pop(b["tool_use_id"])
                            f = re.search(r"Output is being written to: (\S+?\.output)", s)
                            if f:
                                self.output_files[m.group(1)] = f.group(1)
                elif t == "system" and d.get("subtype") == "task_notification":
                    self.running.pop(d.get("task_id"), None)
                    if d.get("output_file"):
                        self.output_files[d["task_id"]] = d["output_file"]
                elif t == "system" and d.get("subtype") == "background_tasks_changed":
                    live = {x.get("task_id") for x in d.get("tasks", [])}
                    for tid in list(self.running):
                        if tid not in live:
                            self.running.pop(tid)
        self.out.flush()

    def only_waiting(self, quiet):
        """The agent ended its turn and has nothing running but `cases wait`:
        with the human's script done, nothing will wake it."""
        with self.lock:
            return (self.last_type == "result" and not self.fg_waits and bool(self.running)
                    and all(any(s == "wait" for s, _ in cases_segments(c)) for c in self.running.values())
                    and time.time() - self.last_event > quiet)

    def idle(self, quiet):
        """The agent ended its turn with nothing running that could wake it."""
        with self.lock:
            return (self.last_type == "result" and not self.running and not self.fg_waits
                    and time.time() - self.last_event > quiet)

    def stop(self):
        try:
            self.proc.stdin.close()
        except OSError:
            pass
        try:
            self.proc.wait(timeout=20)
        except subprocess.TimeoutExpired:
            pass
        try:
            os.killpg(self.proc.pid, signal.SIGKILL)
        except (ProcessLookupError, PermissionError):
            pass
        self.reader.join(timeout=5)
        self.out.close()


class Human:
    """Plays the human's side of a task from its script of steps.

    Steps fire in order, each once. A step waits until a case matches, then,
    unless it says otherwise, until the agent has a `cases wait` running (or
    `fallback` seconds have passed), then `delay` more seconds, and runs its
    command as the human.
    """

    def __init__(self, store, steps, session, log_path, variables, any_order=False):
        self.store = store
        self.steps = list(steps)
        self.session = session
        self.log = open(log_path, "w", encoding="utf-8")
        self.vars = variables
        self.any_order = any_order
        self.waits_seen = 0
        self.acted = {}          # case id -> revision after the human's last write to it
        self.done = 0
        self.errors = []
        self.crashed = None      # the harness's own error, when the script itself broke
        self.stop_flag = threading.Event()
        self.thread = threading.Thread(target=self._loop, daemon=True)

    def start(self):
        self.thread.start()

    def _match(self, step, case):
        m = step.get("match", {})
        kinds = m.get("kind")
        kinds = kinds if isinstance(kinds, list) else [kinds] if kinds else None
        if case.get("kind") == "message" and "message" not in (kinds or []):
            return False
        if kinds and case.get("kind") not in kinds:
            return False
        for key in ("state", "worker"):
            want = m.get(key)
            if want is not None and case.get(key) not in (want if isinstance(want, list) else [want]):
                return False
        if "id" in m and case["id"] != sub_vars(m["id"], self.vars):
            return False
        if "label" in m and m["label"] not in (case.get("labels") or []):
            return False
        if "title_re" in m and not re.search(m["title_re"], case.get("title", ""), re.I):
            return False
        if "last_event" in m:
            want = m["last_event"]
            if (case.get("events") or [{}])[-1].get("event") not in (want if isinstance(want, list) else [want]):
                return False
        if step.get("fresh", True) and case["id"] in self.acted and case["revision"] <= self.acted[case["id"]]:
            return False
        return True

    def _write(self, entry):
        entry["t"] = round(time.time(), 2)
        self.log.write(json.dumps(entry) + "\n")
        self.log.flush()

    def _loop(self):
        # An exception here would end the thread silently and the run would
        # time out as if the agent had stalled, so it is recorded instead.
        try:
            self._steps()
        except Exception as e:
            self.crashed = "%s: %s" % (type(e).__name__, e)
            self._write({"harness_error": self.crashed})

    def _steps(self):
        pending = list(enumerate(self.steps))
        matched_at = {}
        while pending and not self.stop_flag.is_set():
            # In order, only the first pending step may fire; in any order,
            # whichever matches first, as when the agent asks one at a time.
            candidates = pending if self.any_order else pending[:1]
            # Steps keyed to the start or to a wait match no case, so the
            # store is listed only when a candidate needs it.
            needs_cases = any(step.get("at") not in ("start", "waiting") for _, step in candidates)
            cases = self.store.cases() if needs_cases else []
            fired = None
            for idx, step in candidates:
                if step.get("at") in ("start", "waiting"):
                    case = {"id": ""}
                else:
                    case = next((c for c in cases if self._match(step, c)), None)
                if case is None:
                    continue
                since = matched_at.setdefault(idx, time.time())
                if step.get("at") == "start" or not step.get("after_wait", True):
                    ready = True
                elif step.get("at") == "waiting":
                    # A wait started since the human last acted, so each
                    # message goes to a fresh wait.
                    ready = (self.session.waiting() and self.session.waits_started > self.waits_seen) \
                        or time.time() - since > step.get("fallback", 25)
                else:
                    ready = self.session.waiting() or time.time() - since > step.get("fallback", 25)
                if ready:
                    fired = (idx, step, case)
                    break
            if fired is None:
                time.sleep(POLL)
                continue
            pending.remove(fired[:2])
            self._fire(fired[1], fired[2])

    def _fire(self, step, case):
        time.sleep(step.get("delay", 0))
        self.waits_seen = self.session.waits_started
        # The human's own work in the workspace, such as running a command
        # the agent asked them to.
        for rel, content in (step.get("write") or {}).items():
            with open(os.path.join(self.vars["ws"], rel), "w", encoding="utf-8") as f:
                f.write(content)
        for rel in step.get("remove") or []:
            path = os.path.join(self.vars["ws"], rel)
            if os.path.isdir(path):
                shutil.rmtree(path)
            elif os.path.exists(path):
                os.remove(path)
        env = dict(self.vars, id=case["id"])
        # The agent numbers the options and names the rows, so the script
        # picks them by their text.
        if "choose" in step:
            opts = case.get("options") or []
            hit = [i for i, o in enumerate(opts, 1) if re.search(step["choose"], o, re.I)]
            env["choice"] = str(hit[0]) if hit else "1"
        if "rows" in step:
            verdicts = []
            for row in case.get("rows") or []:
                text = " ".join(str(row.get(k, "")) for k in ("id", "label", "script"))
                verdict = "hold"
                for pat, v in step["rows"].items():
                    if re.search(pat, text, re.I):
                        verdict = v
                        break
                verdicts += ["--row", f"{row['id']}={verdict}"]
            env["rows"] = verdicts
        if step.get("save"):
            self.vars[step["save"]] = case["id"]
        argv = []
        for word in step.get("do_" + str(case.get("kind")), step["do"]):
            if word == "{rows}":
                argv += env.get("rows", [])
            else:
                argv.append(sub_vars(word, {k: v for k, v in env.items() if isinstance(v, str)}))
        if argv[0] in ("answer", "park", "resume", "withdraw", "message", "amend") and "--as" not in argv:
            argv += ["--as", HUMAN]
        waiting = self.session.waiting()
        p = self.store.run(argv + ["--json"])
        entry = {"step": self.done, "do": argv, "case": case["id"], "exit": p.returncode,
                 "stdout": p.stdout.strip()[:500], "stderr": p.stderr.strip()[:500],
                 "agent_waiting": waiting}
        if p.returncode == 0:
            try:
                out = json.loads(p.stdout)
                self.acted[out["id"]] = out["revision"]
                if step.get("save_new"):
                    self.vars[step["save_new"]] = out["id"]
            except (json.JSONDecodeError, KeyError):
                pass
        elif not step.get("may_fail"):
            self.errors.append(entry)
        self._write(entry)
        self.done += 1

    def stop(self):
        self.stop_flag.set()
        self.thread.join(timeout=5)
        self.log.close()


# Passed through to every claude session when set. On the laptop auth lives in
# the keychain under HOME; a cloud container gives it in the environment and
# may reach the API only through a proxy with its own CA.
PASS_VARS = (
    "ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_BASE_URL", "CLAUDE_CODE_OAUTH_TOKEN",
    "HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "NO_PROXY", "no_proxy",
    "SSL_CERT_FILE", "NODE_EXTRA_CA_CERTS",
)


def base_env(*names):
    """names from the environment, plus PASS_VARS, where set."""
    return {k: os.environ[k] for k in names + PASS_VARS if k in os.environ}


def make_env(ws, bindir):
    """The environment of the session and of every command the harness runs."""
    env = base_env("HOME", "USER", "LOGNAME", "LANG", "TERM")
    env["PATH"] = bindir + ":/usr/bin:/bin:/usr/sbin:/sbin"
    env["CASES_STORE"] = os.path.join(ws, ".cases", "cases.db")
    for var, sub in (("XDG_CONFIG_HOME", "config"), ("XDG_DATA_HOME", "data"), ("XDG_STATE_HOME", "state")):
        env[var] = os.path.join(ws, ".cases", sub)
        os.makedirs(env[var], exist_ok=True)
    return env


SETTINGS = {
    "sandbox": {"enabled": True, "autoAllowBashIfSandboxed": True, "allowUnsandboxedCommands": False},
}
ALLOWED = "Bash,Read,Write,Edit,Glob,Grep,Skill"


def session_argv(claude, *, arm, model, effort, plugin_dir, max_turns=60):
    """The command line of an agent session: only project and local settings,
    the sandbox on, and the plugin with the skill only in the with arm."""
    argv = [claude, "-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose",
            "--setting-sources", "project,local", "--no-session-persistence",
            "--model", model, "--max-turns", str(max_turns),
            "--settings", json.dumps(SETTINGS),
            "--tools", ALLOWED, "--allowedTools", ALLOWED]
    if effort:
        argv += ["--effort", effort]
    if arm == "with":
        argv += ["--plugin-dir", plugin_dir]
    return argv


def rate_limit_peak(transcript_path):
    """The highest utilisation the account reported during a session, and
    whether any request was refused, from its rate_limit_event lines."""
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


def run_task(task, *, arm, binary, claude, plugin_dir, model, effort, out_dir, max_turns=60):
    """Runs one task once. Writes transcript.jsonl, human.jsonl, store.json,
    tasks/<id>.output and run.json into out_dir, and returns run.json's content."""
    os.makedirs(out_dir, exist_ok=True)
    ws = tempfile.mkdtemp(prefix="cases-eval-")
    ws = os.path.realpath(ws)
    bindir = os.path.join(ws, ".bin")
    os.makedirs(bindir)
    shutil.copy2(binary, os.path.join(bindir, "cases"))
    env = make_env(ws, bindir)
    store = Store(os.path.join(bindir, "cases"), env["CASES_STORE"], env)
    info = {"task": task["id"], "arm": arm, "model": model, "effort": effort, "workspace": ws,
            "infra_error": None, "timed_out": False}
    t0 = time.time()
    try:
        for rel, content in (task.get("files") or {}).items():
            path = os.path.join(ws, rel)
            os.makedirs(os.path.dirname(path), exist_ok=True)
            with open(path, "w", encoding="utf-8") as f:
                f.write(content)
            if rel.endswith(".sh"):
                os.chmod(path, 0o755)
        variables = {"ws": ws}
        for seed in task.get("seed", []):
            argv = sub_vars(seed["args"], variables)
            if seed.get("as") == "human" and "--as" not in argv:
                argv += ["--as", HUMAN]
            p = store.run(argv, check=True)
            if seed.get("save"):
                text = p.stdout.strip()
                variables[seed["save"]] = json.loads(text)["id"] if text.startswith("{") else text.split()[0]
            if seed.get("sleep"):
                time.sleep(seed["sleep"])
        info["seeded"] = [c["id"] for c in store.cases()]
        prompt = sub_vars(task["prompt"], variables)
        argv = session_argv(claude, arm=arm, model=model, effort=effort, plugin_dir=plugin_dir,
                            max_turns=max_turns)
        session = Session(argv, env, ws, os.path.join(out_dir, "transcript.jsonl"))
        human = Human(store, task.get("human", []), session, os.path.join(out_dir, "human.jsonl"), variables,
                      any_order=task.get("human_order") == "any")
        human.start()
        session.send(prompt)
        limit = t0 + 60 * task.get("max_minutes", 10)
        while True:
            if session.proc.poll() is not None:
                break
            if session.idle(quiet=4):
                break
            if human.crashed:
                info["infra_error"] = "human script: " + human.crashed
                break
            if human.done == len(task.get("human", [])) and session.only_waiting(quiet=15):
                info["ended_waiting"] = True
                break
            if time.time() > limit:
                info["timed_out"] = True
                break
            time.sleep(0.5)
        human.stop()
        session.stop()
        info["human_steps_done"] = human.done
        info["human_steps"] = len(task.get("human", []))
        info["human_errors"] = human.errors
        info["session_exit"] = session.proc.returncode
        info["results"] = session.results
        info["waits_started"] = session.waits_started
        tdir = os.path.join(out_dir, "tasks")
        os.makedirs(tdir, exist_ok=True)
        for tid, path in session.output_files.items():
            if os.path.exists(path):
                shutil.copy2(path, os.path.join(tdir, tid + ".output"))
        with open(os.path.join(out_dir, "store.json"), "w", encoding="utf-8") as f:
            json.dump(store.cases(), f, indent=1)
        # The workspace as the agent left it, for the graders.
        listing = []
        for root, dirs, files in os.walk(ws):
            dirs[:] = [d for d in dirs if d not in (".cases", ".bin", ".git")]
            for name in files:
                rel = os.path.relpath(os.path.join(root, name), ws)
                listing.append(rel)
                dst = os.path.join(out_dir, "workspace", rel)
                os.makedirs(os.path.dirname(dst), exist_ok=True)
                shutil.copy2(os.path.join(ws, rel), dst)
        info["files"] = sorted(listing)
    except Exception as e:  # anything that is the harness's fault, not the agent's
        info["infra_error"] = "%s: %s" % (type(e).__name__, e)
    info["seconds"] = round(time.time() - t0, 1)
    with open(os.path.join(out_dir, "run.json"), "w", encoding="utf-8") as f:
        json.dump(info, f, indent=1)
    shutil.rmtree(ws, ignore_errors=True)
    return info

