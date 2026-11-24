// What the composer (static/compose.ts, IN-13) does once it is open: its
// effects, and what each of its handlers does to it. render-to-string runs
// neither, so these tests give compose.ts a stand-in for preact/hooks
// (hooks-stand-in.js), call Composer as a function, hand its refs stand-in
// elements that log what is done to them, and call the handlers in the tree
// it gives. The tree is drawn to HTML with render-to-string to see what the
// composer shows.
//
// The stand-in is in place only if this file loads compose.ts first, so it
// needs a process of its own, as `node --test` gives each file by default;
// it sets globalThis.window and globalThis.fetch for the same reason.
import { test } from "node:test";
import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import { render } from "preact-render-to-string";

const standIn = new URL("./hooks-stand-in.js", import.meta.url).href;
registerHooks({
  resolve(specifier, context, next) {
    if (specifier === "preact/hooks" && context.parentURL?.endsWith("/static/compose.ts")) return next(standIn, context);
    return next(specifier, context);
  },
});
const { mount } = await import("./hooks-stand-in.js");
const { Composer } = await import("../static/compose.ts");

const now = Date.parse("2026-09-27T12:00:00Z");
const at = (minutes) => new Date(now - minutes * 60000).toISOString();
const listener = (worker, labels) => ({ worker, labels, waits: [{ labels, kinds: [] }], since: at(90), heard: at(0.1) });
const kase = (id, worker, labels, minutes) => ({ id, kind: "decision", state: "open", urgency: "today", title: `case ${id}`, worker, labels, updated_at: at(minutes), events: [] });

// Every vnode of `tree`, in order.
function* nodes(tree) {
  if (Array.isArray(tree)) {
    for (const child of tree) yield* nodes(child);
    return;
  }
  if (!tree || typeof tree !== "object") return;
  yield tree;
  yield* nodes(tree.props?.children);
}

// The text of a vnode's children, where they are strings.
const text = (v) => [...nodes(v)].flatMap((n) => [n.props?.children].flat().filter((c) => typeof c === "string")).join("");

// The composer, open with `props`, its refs holding stand-ins that log what
// is done to them in `log`. `dialog: false` leaves the dialog's ref empty;
// `picker` makes the picker's stand-in. `viewport` is window.visualViewport.
function open(props = {}, { dialog: hasDialog = true, opened = false, picker, viewport } = {}) {
  const log = [];
  const sent = [];
  globalThis.window = viewport ? { visualViewport: viewport } : {};
  const back = { current: null };
  const dialog = {
    open: opened,
    style: { setProperty: (name, value) => log.push(`${name}: ${value}`) },
    showModal() {
      log.push("showModal");
      this.open = true;
    },
    close() {
      log.push("close");
      this.open = false;
    },
    querySelector: (selector) => ({ focus: () => log.push(`focus ${selector}`) }),
  };
  const all = { to: "", re: "", listeners: [], cases: [], now, onSent: (c) => sent.push(c), onClose: () => log.push("onClose"), back, ...props };
  const attach = (tree) => {
    for (const v of nodes(tree)) {
      if (v.type === "dialog" && hasDialog) v.ref.current = dialog;
      if (v.type === "fieldset") v.ref.current = (picker ?? radios)(v, log);
    }
  };
  const c = mount(Composer, all, attach);
  const composer = {
    log,
    sent,
    back,
    get tree() {
      return c.tree;
    },
    page: () => render(c.tree),
    // The first vnode `pick` says yes to.
    find(pick, what = "a vnode") {
      const v = [...nodes(c.tree)].find(pick);
      assert.ok(v, `no ${what}: ${render(c.tree)}`);
      return v;
    },
    // Does `fn` to it, and draws it again.
    act(fn) {
      fn();
      c.draw();
      return composer;
    },
    unmount: () => c.unmount(),
  };
  return composer;
}

// The picker's stand-in: its radios, as the page would have them, from its
// choices and the "another" radio, in order. Focusing one logs its value.
function radios(fieldset, log) {
  const list = [...nodes(fieldset)]
    .filter((v) => (v.type === "input" && v.props.name === "to") || v.props?.onPick)
    .map((v) => {
      const [value, checked] = v.props.onPick ? [v.props.p.key, v.props.checked] : [v.props.value, v.props.checked];
      return { value, checked, focus: () => log.push(`focus ${value}`) };
    });
  return { querySelectorAll: () => list };
}

const button = (label) => (v) => v.type === "button" && text(v).startsWith(label);
const event = (extra = {}) => {
  const e = { prevented: false, preventDefault() { this.prevented = true; }, ...extra };
  return e;
};
const note = (c, body) => c.act(() => c.find((v) => v.props?.name === "body" && v.props.onInput, "note").props.onInput(body));

test("opened, the composer shows itself as a modal with the note focused, once", () => {
  const c = open();
  assert.deepEqual(c.log, ["showModal", 'focus textarea[name="body"]']);
  note(c, "a");
  assert.deepEqual(c.log, ["showModal", 'focus textarea[name="body"]']);
  // A dialog shown already is not shown again; one not there yet is left.
  assert.deepEqual(open({}, { opened: true }).log, ['focus textarea[name="body"]']);
  assert.deepEqual(open({}, { dialog: false }).log, []);
});

test("asked for, the picker takes the focus: its choice, or its first", () => {
  const listeners = [listener("bun-pins", [])];
  const cases = [kase("a1", "ghost", [], 5)];
  // Opened for no one, the picker shows from the start and the note keeps the focus.
  assert.ok(!open({ listeners, cases }).log.some((l) => l.startsWith("focus ") && !l.includes("textarea")));
  const c = open({ to: "ghost", listeners, cases });
  assert.ok(!c.page().includes('<fieldset class="picker">'));
  c.act(() => c.find(button("change"), "change button").props.onClick());
  assert.ok(c.page().includes('<fieldset class="picker">'), c.page());
  assert.deepEqual(c.log.slice(2), ["focus worker:ghost"]);
  // Once: typing elsewhere, or picking another, leaves the focus where it is.
  note(c, "Rebase");
  c.act(() => c.find((v) => v.type === "input" && v.props.value === "other").props.onClick());
  c.act(() => c.find((v) => v.props?.name === "other").props.onInput({ target: { value: "o" } }));
  assert.deepEqual(c.log.slice(2), ["focus worker:ghost"]);
  // Opened for a name no one has, it is another, picked.
  const other = open({ to: "nobody", listeners, cases });
  other.act(() => other.find(button("change")).props.onClick());
  assert.deepEqual(other.log.slice(2), ["focus other"]);
  // With none checked, the first; with no picker drawn, none.
  const first = open({ to: "ghost", listeners, cases }, { picker: (f, log) => ({ querySelectorAll: () => radios(f, log).querySelectorAll().map((r) => ({ ...r, checked: false })) }) });
  first.act(() => first.find(button("change")).props.onClick());
  assert.deepEqual(first.log.slice(2), ["focus worker:bun-pins"]);
  const none = open({ to: "ghost", listeners, cases }, { picker: () => null });
  none.act(() => none.find(button("change")).props.onClick());
  assert.deepEqual(none.log.slice(2), []);
});

test("Escape closes the composer, or with a note written asks first, with the focus on keeping it", () => {
  const cancel = (c) => {
    const e = event();
    c.act(() => c.find((v) => v.type === "dialog").props.onCancel(e));
    assert.equal(e.prevented, true);
  };
  const empty = open();
  cancel(empty);
  assert.deepEqual(empty.log.slice(2), ["close"]);
  const blank = open();
  note(blank, " \n ");
  cancel(blank);
  assert.deepEqual(blank.log.slice(2), ["close"]);

  const c = open();
  note(c, "Rebase");
  cancel(c);
  assert.deepEqual(c.log.slice(2), ["focus .leaving .keep"]);
  // Once: typing on while it asks leaves the focus where it is.
  note(c, "Rebase!");
  assert.deepEqual(c.log.slice(2), ["focus .leaving .keep"]);
  assert.ok(c.page().includes('<div class="actions leaving" role="alert"><span>discard this note?</span>'), c.page());
  assert.ok(!c.page().includes('type="submit"'), c.page());
  // Keeping it gives the note the focus back, and the send button.
  c.act(() => c.find(button("keep writing")).props.onClick());
  assert.deepEqual(c.log.slice(3), ['focus textarea[name="body"]']);
  assert.ok(c.page().includes('type="submit"') && !c.page().includes("discard this note?"), c.page());
  // Asked again, Escape once more closes it, as discard does.
  cancel(c);
  cancel(c);
  assert.deepEqual(c.log.slice(4), ["focus .leaving .keep", "close"]);
  const d = open();
  note(d, "Rebase");
  cancel(d);
  d.act(() => d.find(button("discard")).props.onClick());
  assert.equal(d.log.at(-1), "close");
  // The close button does what Escape does.
  const e = open();
  e.act(() => e.find(button("close")).props.onClick());
  assert.equal(e.log.at(-1), "close");
  // A close of the dialog is the app's to hear.
  const onClose = () => {};
  assert.equal(open({ onClose }).find((v) => v.type === "dialog").props.onClose, onClose);
  c.find((v) => v.type === "dialog").props.onClose();
  assert.equal(c.log.at(-1), "onClose");
});

test("a click on the backdrop closes the composer, but not one that started inside it", () => {
  const c = open();
  const dialog = () => c.find((v) => v.type === "dialog").props;
  const on = {};
  const inside = {};
  // Pressed and let go on the backdrop: it closes.
  c.act(() => dialog().onPointerDown({ target: on, currentTarget: on }));
  c.act(() => dialog().onClick({ target: on, currentTarget: on }));
  assert.deepEqual(c.log.slice(2), ["close"]);
  // Let go there a second time with no press: no close.
  c.act(() => dialog().onClick({ target: on, currentTarget: on }));
  // Pressed inside and let go outside, as a drag is: no close.
  c.act(() => dialog().onPointerDown({ target: inside, currentTarget: on }));
  c.act(() => dialog().onClick({ target: on, currentTarget: on }));
  // Pressed on the backdrop and let go inside: no close. The browser sends
  // the click to the dialog, the nearest element both were in.
  c.act(() => dialog().onPointerDown({ target: on, currentTarget: on }));
  c.act(() => dialog().onPointerUp?.({ target: inside, currentTarget: on }));
  c.act(() => dialog().onClick({ target: on, currentTarget: on }));
  assert.deepEqual(c.log.slice(2), ["close"]);
});

test("a press on the backdrop let go inside the composer, as a browser sends it, does not close it", () => {
  const c = open();
  const dialog = () => c.find((v) => v.type === "dialog").props;
  const on = {};
  const inside = {};
  // Pressed on the backdrop, let go on the note: the click goes to the
  // dialog, which both are in. No close.
  c.act(() => dialog().onPointerDown({ target: on, currentTarget: on }));
  c.act(() => dialog().onPointerUp?.({ target: inside, currentTarget: on }));
  c.act(() => dialog().onClick({ target: on, currentTarget: on }));
  assert.deepEqual(c.log.slice(2), []);
  // Pressed and let go on the backdrop, every event as a browser sends it:
  // it closes.
  c.act(() => dialog().onPointerDown({ target: on, currentTarget: on }));
  c.act(() => dialog().onPointerUp?.({ target: on, currentTarget: on }));
  c.act(() => dialog().onClick({ target: on, currentTarget: on }));
  assert.deepEqual(c.log.slice(2), ["close"]);
});

test("a press inside the composer let go on the backdrop, or a later press inside, does not close it", () => {
  const c = open();
  const dialog = () => c.find((v) => v.type === "dialog").props;
  const on = {};
  const inside = {};
  // Pressed inside, let go on the backdrop: the click goes to the dialog.
  c.act(() => dialog().onPointerDown({ target: inside, currentTarget: on }));
  c.act(() => dialog().onPointerUp({ target: on, currentTarget: on }));
  c.act(() => dialog().onClick({ target: on, currentTarget: on }));
  // Pressed on the backdrop, then inside with no click between: no close.
  c.act(() => dialog().onPointerDown({ target: on, currentTarget: on }));
  c.act(() => dialog().onPointerDown({ target: inside, currentTarget: on }));
  c.act(() => dialog().onClick({ target: on, currentTarget: on }));
  assert.deepEqual(c.log.slice(2), []);
});

test("a press on the composer's backdrop the browser cancels is no click on it", () => {
  const c = open();
  const dialog = () => c.find((v) => v.type === "dialog").props;
  const on = {};
  c.act(() => dialog().onPointerDown({ target: on, currentTarget: on }));
  c.act(() => dialog().onPointerCancel({ target: on, currentTarget: on }));
  c.act(() => dialog().onClick({ target: on, currentTarget: on }));
  assert.deepEqual(c.log.slice(2), []);
});

test("a click on the backdrop with a note written asks first, as Escape does", () => {
  const c = open();
  note(c, "Rebase");
  const dialog = () => c.find((v) => v.type === "dialog").props;
  const on = {};
  c.act(() => dialog().onPointerDown({ target: on, currentTarget: on }));
  c.act(() => dialog().onPointerUp({ target: on, currentTarget: on }));
  c.act(() => dialog().onClick({ target: on, currentTarget: on }));
  assert.ok(c.page().includes("discard this note?"), c.page());
  assert.ok(!c.log.includes("close"));
});

test("back closes the composer, or with a note written asks and says it asked", () => {
  const c = open();
  assert.equal(c.back.current(), false);
  assert.deepEqual(c.log.slice(2), ["close"]);
  const d = open();
  note(d, "Rebase");
  let asked;
  d.act(() => (asked = d.back.current()));
  assert.equal(asked, true);
  assert.ok(d.page().includes("discard this note?"));
  assert.ok(!d.log.includes("close"));
});

test("a plain click on the case a reply is about asks first when a note is written", () => {
  const cases = [{ ...kase("UkLWZ", "ghost", [], 5), title: "Pin bun?" }];
  const c = open({ to: "ghost", re: "UkLWZ", cases });
  const about = () => c.find((v) => v.type === "a").props.onClick;
  const click = (extra = {}) => {
    const e = event({ button: 0, metaKey: false, ctrlKey: false, shiftKey: false, altKey: false, ...extra });
    c.act(() => about()(e));
    return e.prevented;
  };
  // No note: the app goes to the case, and the composer with it.
  assert.equal(click(), false);
  note(c, "Rebase");
  // Another button, or a modifier, opens it elsewhere: the dialog stays.
  for (const extra of [{ button: 1 }, { metaKey: true }, { ctrlKey: true }, { shiftKey: true }, { altKey: true }]) {
    assert.equal(click(extra), false, JSON.stringify(extra));
  }
  assert.ok(!c.page().includes("discard this note?"));
  assert.equal(click(), true);
  assert.ok(c.page().includes("discard this note?"));
  // About a case not in the list: its id, at its address.
  assert.ok(open({ to: "ghost", re: "p6klV" }).page().includes('<p class="re">about <a href="/cases/p6klV">p6klV</a></p>'));
});

// A stand-in for serve: GET /v1 gives the token; each POST takes the next
// of `answers`, a function giving its Response. The POSTs' bodies are in
// `posted`.
function serve(answers) {
  const posted = [];
  globalThis.fetch = async (url, init) => {
    if (url === "/v1") return new Response(JSON.stringify({ csrf: "t" }), { status: 200 });
    posted.push([url, JSON.parse(init.body)]);
    return answers.shift()();
  };
  return posted;
}

const submit = async (c) => {
  const e = event();
  await c.find((v) => v.type === "form").props.onSubmit(e);
  c.act(() => {});
  assert.equal(e.prevented, true);
};

test("send says what is missing, and sends nothing", async () => {
  const posted = serve([]);
  const c = open({ to: "ghost", cases: [kase("a1", "ghost", [], 5)] });
  await submit(c);
  assert.ok(c.page().includes('<p class="error" role="alert">write a note</p>'), c.page());
  assert.deepEqual(posted, []);
});

test("a message sent closes the composer and says so; one refused says why and can be sent again", async () => {
  const after = { id: "m1", kind: "message" };
  const posted = serve([
    () => new Response(JSON.stringify({ error: { category: "request", message: "no such worker" } }), { status: 400 }),
    () => new Response(JSON.stringify(after), { status: 200 }),
  ]);
  const c = open({ to: "ghost", re: "a1", cases: [kase("a1", "ghost", [], 5)] });
  note(c, "Rebase");
  await submit(c);
  assert.ok(c.page().includes('<p class="error" role="alert">no such worker</p>'), c.page());
  assert.ok(!c.log.includes("close"));
  await submit(c);
  assert.deepEqual(posted, [
    ["/v1/cases", { kind: "message", urgency: "today", title: "Rebase", body: "Rebase", worker: "ghost", re: "a1" }],
    ["/v1/cases", { kind: "message", urgency: "today", title: "Rebase", body: "Rebase", worker: "ghost", re: "a1" }],
  ]);
  assert.equal(c.log.at(-1), "close");
  assert.deepEqual(c.sent, [after]);
  assert.ok(c.page().includes('<p class="error" role="alert" hidden></p>'), c.page());
  // Sent, it is not sent again.
  await submit(c);
  assert.equal(posted.length, 2);
});

test("a send not answered yet keeps a second from going, for ten seconds", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let answer;
  const posted = serve([
    () => new Promise((resolve) => (answer = resolve)),
    () => new Response(JSON.stringify({ id: "m2" }), { status: 200 }),
    () => new Response(JSON.stringify({ id: "m3" }), { status: 200 }),
  ]);
  const c = open({ to: "ghost", cases: [kase("a1", "ghost", [], 5)] });
  note(c, "Rebase");
  const form = c.find((v) => v.type === "form").props;
  const first = form.onSubmit(event());
  await new Promise((resolve) => setImmediate(resolve));
  await form.onSubmit(event());
  assert.equal(posted.length, 1);
  // After ten seconds, and not before, a send goes again (IN-09).
  t.mock.timers.tick(9999);
  await form.onSubmit(event());
  assert.equal(posted.length, 1);
  t.mock.timers.tick(1);
  await form.onSubmit(event());
  assert.equal(posted.length, 2);
  assert.deepEqual(c.sent, [{ id: "m2" }]);
  // The first send's answer, come last, changes nothing: the composer has
  // sent, says no refusal, and sends nothing more.
  answer(new Response(JSON.stringify({ error: { category: "request", message: "late" } }), { status: 400 }));
  await first;
  c.act(() => {});
  assert.ok(!c.page().includes("late"), c.page());
  await form.onSubmit(event());
  assert.equal(posted.length, 2);
});

test("a send taken after ten seconds holds the guard for good", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let answer;
  const posted = serve([() => new Promise((resolve) => (answer = resolve)), () => new Response(JSON.stringify({ id: "m2" }), { status: 200 })]);
  const c = open({ to: "ghost", cases: [kase("a1", "ghost", [], 5)] });
  note(c, "Rebase");
  const form = c.find((v) => v.type === "form").props;
  const first = form.onSubmit(event());
  await new Promise((resolve) => setImmediate(resolve));
  t.mock.timers.tick(10000);
  answer(new Response(JSON.stringify({ id: "m1" }), { status: 200 }));
  await first;
  assert.deepEqual(c.sent, [{ id: "m1" }]);
  await form.onSubmit(event());
  assert.equal(posted.length, 1);
});

test("a send taken after a later send went counts, and the later send's answer changes nothing", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const answers = [];
  const posted = serve([
    () => new Promise((resolve) => answers.push(resolve)),
    () => new Promise((resolve) => answers.push(resolve)),
    () => new Response(JSON.stringify({ id: "m3" }), { status: 200 }),
  ]);
  const c = open({ to: "ghost", cases: [kase("a1", "ghost", [], 5)] });
  note(c, "Rebase");
  const form = c.find((v) => v.type === "form").props;
  const first = form.onSubmit(event());
  await new Promise((resolve) => setImmediate(resolve));
  t.mock.timers.tick(10000);
  const second = form.onSubmit(event());
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(posted.length, 2);
  // The first is taken while the second is out: the message is sent, so
  // the composer goes, and sends nothing more.
  answers[0](new Response(JSON.stringify({ id: "m1" }), { status: 200 }));
  await first;
  c.act(() => {});
  assert.deepEqual([c.sent, c.log.at(-1)], [[{ id: "m1" }], "close"]);
  // The second's refusal, come after, changes nothing.
  answers[1](new Response(JSON.stringify({ error: { category: "request", message: "no" } }), { status: 400 }));
  await second;
  c.act(() => {});
  assert.ok(!c.page().includes('role="alert">no<'), c.page());
  await form.onSubmit(event());
  assert.equal(posted.length, 2);
});

test("a composer closed with its send out keeps the next composer from sending until that send is answered", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let answer;
  const posted = serve([() => new Promise((resolve) => (answer = resolve)), () => new Response(JSON.stringify({ id: "m2" }), { status: 200 })]);
  const a = open({ to: "ghost", cases: [kase("a1", "ghost", [], 5)] });
  note(a, "Rebase");
  const first = a.find((v) => v.type === "form").props.onSubmit(event());
  await new Promise((resolve) => setImmediate(resolve));
  // Escape, then discard: the dialog closes with the send still out, and
  // the app takes the composer away, as preact does, its refs emptied.
  a.act(() => a.find((v) => v.type === "dialog").props.onCancel(event()));
  a.act(() => a.find(button("discard")).props.onClick());
  assert.equal(a.log.at(-1), "close");
  a.find((v) => v.type === "dialog").ref.current = null;
  a.unmount();
  // Opened again, with the same note: its send is held, and says why.
  const b = open({ to: "ghost", cases: [kase("a1", "ghost", [], 5)] });
  note(b, "Rebase");
  await submit(b);
  assert.equal(posted.length, 1);
  assert.ok(b.page().includes('<p class="error" role="alert">the message sent before is not answered yet: wait for it before sending this one</p>'), b.page());
  // The first is taken: the line says it was sent, which it was, and the
  // open composer stays open, and says so too, since the line is behind it.
  answer(new Response(JSON.stringify({ id: "m1" }), { status: 200 }));
  await first;
  b.act(() => {});
  assert.deepEqual([a.sent, b.sent], [[{ id: "m1" }], []]);
  assert.ok(!b.log.includes("close"));
  assert.ok(b.page().includes('<p class="error" role="alert">the message sent before was sent: send this one only if it is another message</p>'), b.page());
  // Answered, it holds the next composer no longer: a send from it is the
  // human's, made after the composer said the first was sent.
  await submit(b);
  assert.equal(posted.length, 2);
  assert.deepEqual(b.sent, [{ id: "m2" }]);
});

test("a composer closed with its send out says why that send was refused in the next one, which then sends", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let answer;
  const posted = serve([() => new Promise((resolve) => (answer = resolve)), () => new Response(JSON.stringify({ id: "m2" }), { status: 200 })]);
  const a = open({ to: "ghost", cases: [kase("a1", "ghost", [], 5)] });
  note(a, "Rebase");
  const first = a.find((v) => v.type === "form").props.onSubmit(event());
  await new Promise((resolve) => setImmediate(resolve));
  a.unmount();
  const b = open({ to: "ghost", cases: [kase("a1", "ghost", [], 5)] });
  note(b, "Rebase");
  await submit(b);
  assert.equal(posted.length, 1);
  answer(new Response(JSON.stringify({ error: { category: "request", message: "no one is ghost" } }), { status: 400 }));
  await first;
  b.act(() => {});
  assert.deepEqual(a.sent, []);
  assert.ok(b.page().includes('<p class="error" role="alert">about the message sent before: no one is ghost</p>'), b.page());
  await submit(b);
  assert.equal(posted.length, 2);
  assert.deepEqual(b.sent, [{ id: "m2" }]);
});

test("a composer closed with its send out holds the next one for that send's ten seconds only", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let answer;
  const posted = serve([() => new Promise((resolve) => (answer = resolve)), () => new Response(JSON.stringify({ id: "m2" }), { status: 200 })]);
  const a = open({ to: "ghost", cases: [kase("a1", "ghost", [], 5)] });
  note(a, "Rebase");
  const first = a.find((v) => v.type === "form").props.onSubmit(event());
  await new Promise((resolve) => setImmediate(resolve));
  a.unmount();
  const b = open({ to: "ghost", cases: [kase("a1", "ghost", [], 5)] });
  note(b, "Rebase");
  t.mock.timers.tick(9999);
  await submit(b);
  assert.equal(posted.length, 1);
  // After ten seconds a send goes again, as from the composer that sent
  // (IN-09).
  t.mock.timers.tick(1);
  await submit(b);
  assert.equal(posted.length, 2);
  assert.deepEqual(b.sent, [{ id: "m2" }]);
  // The first's late refusal says nothing in the composer that has sent
  // since.
  answer(new Response(JSON.stringify({ error: { category: "request", message: "late" } }), { status: 400 }));
  await first;
  b.act(() => {});
  assert.ok(!b.page().includes("late"), b.page());
});

test("a send refused stops its ten seconds, so they do not cut short the next send's", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  let answer;
  const posted = serve([
    () => new Response(JSON.stringify({ error: { category: "request", message: "no" } }), { status: 400 }),
    () => new Promise((resolve) => (answer = resolve)),
    () => new Response(JSON.stringify({ id: "m4" }), { status: 200 }),
  ]);
  const c = open({ to: "ghost", cases: [kase("a1", "ghost", [], 5)] });
  note(c, "Rebase");
  const form = c.find((v) => v.type === "form").props;
  await form.onSubmit(event());
  t.mock.timers.tick(5000);
  const second = form.onSubmit(event());
  await new Promise((resolve) => setImmediate(resolve));
  // Ten seconds after the first send, the second is still under way.
  t.mock.timers.tick(5000);
  await form.onSubmit(event());
  assert.equal(posted.length, 2);
  answer(new Response(JSON.stringify({ error: { category: "request", message: "late" } }), { status: 400 }));
  await second;
});

test("on a phone the dialog takes the part of the screen the keyboard leaves, as it moves", () => {
  const heard = new Map();
  const viewport = {
    offsetTop: 0,
    height: 800,
    addEventListener: (name, fn) => heard.set(name, fn),
    removeEventListener: (name, fn) => heard.get(name) === fn && heard.delete(name),
  };
  const c = open({}, { viewport });
  assert.deepEqual(c.log.slice(2), ["--seen-top: 0px", "--seen-height: 800px"]);
  Object.assign(viewport, { offsetTop: 40, height: 400 });
  heard.get("resize")();
  viewport.offsetTop = 60;
  heard.get("scroll")();
  assert.deepEqual(c.log.slice(4), ["--seen-top: 40px", "--seen-height: 400px", "--seen-top: 60px", "--seen-height: 400px"]);
  c.unmount();
  assert.equal(heard.size, 0);
  // With no dialog, nothing is heard.
  open({}, { dialog: false, viewport: { ...viewport, addEventListener: () => assert.fail("heard") } });
});

test("the fields of the composer make the message it sends", async () => {
  const listeners = [listener("lead", [])];
  const cases = [kase("a1", "docs", ["deploy"], 30), kase("a2", "ghost", [], 30)];
  const after = { id: "m3" };
  const posted = serve([() => new Response(JSON.stringify(after), { status: 200 }), () => new Response(JSON.stringify(after), { status: 200 })]);
  const c = open({ listeners, cases });
  const fold = () => c.find((v) => v.type === "button" && v.props.class === "fold", "fold");
  // The fold opens and closes by hand, and a filter narrows the list.
  assert.equal(fold().props["aria-expanded"], "false");
  c.act(() => fold().props.onClick());
  assert.equal(fold().props["aria-expanded"], "true");
  c.act(() => c.find((v) => v.props?.name === "filter").props.onInput({ target: { value: "zz" } }));
  assert.ok(c.page().includes("no worker or label here has “zz” in its name"), c.page());
  c.act(() => c.find((v) => v.props?.name === "filter").props.onInput({ target: { value: "DEP" } }));
  assert.ok(c.page().includes('<span class="tag">deploy</span>') && !c.page().includes("<strong>docs</strong>"), c.page());
  c.act(() => fold().props.onClick());
  assert.equal(fold().props["aria-expanded"], "false");
  c.act(() => fold().props.onClick());
  // A choice picked by its radio.
  c.act(() => c.find((v) => v.props?.p?.key === "label:deploy").props.onPick("label:deploy"));
  note(c, "Ship it");
  await submit(c);
  assert.deepEqual(posted[0][1].labels, ["deploy"]);

  // Another, by kind and name, picked by a click or a change.
  for (const how of ["onClick", "onChange"]) {
    const d = open({ listeners, cases });
    d.act(() => d.find((v) => v.type === "input" && v.props.value === "other").props[how]());
    d.act(() => d.find((v) => v.type === "select").props.onChange({ target: { value: "label" } }));
    d.act(() => d.find((v) => v.props?.name === "other").props.onInput({ target: { value: " ops " } }));
    assert.ok(d.page().includes('<input type="text" name="other" aria-label="its name" value=" ops "'), d.page());
    note(d, "Ship it");
    if (how === "onClick") {
      await submit(d);
      assert.deepEqual(posted[1][1].labels, ["ops"]);
    }
  }
});

test("a choice's radio picks it, by a click or a change", () => {
  const c = open({ listeners: [listener("lead", [])] });
  const choice = c.find((v) => v.props?.onPick, "choice");
  const picked = [];
  const radio = [...nodes(choice.type({ ...choice.props, onPick: (key) => picked.push(key) }))].find((v) => v.type === "input");
  radio.props.onClick();
  radio.props.onChange();
  assert.deepEqual(picked, ["worker:lead", "worker:lead"]);
});

test("the one picked stays in sight, checked, when its fold is closed by hand", () => {
  const c = open({ to: "ghost", listeners: [listener("lead", [])], cases: [kase("a1", "ghost", [], 30), kase("a2", "docs", [], 30)] });
  c.act(() => c.find(button("change")).props.onClick());
  const fold = () => c.find((v) => v.type === "button" && v.props.class === "fold", "fold");
  // Open while the one picked is among those not listening.
  assert.equal(fold().props["aria-expanded"], "true");
  c.act(() => fold().props.onClick());
  assert.equal(fold().props["aria-expanded"], "false");
  assert.ok(render(fold()).endsWith(">1 more, not listening</button>"), render(fold()));
  const held = c.find((v) => v.props?.onPick && v.props.p.key === "worker:ghost", "the pick");
  assert.equal(held.props.checked, true);
  assert.ok(!c.page().includes("<strong>docs</strong>"), c.page());
});
