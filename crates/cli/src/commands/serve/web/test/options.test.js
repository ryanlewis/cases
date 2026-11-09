// The options dialog, the keys dialog, and the desktop notifications they
// turn on (static/options.ts: IN-08, IN-09, IN-10). These tests pin what the
// module does today.
//
// options.ts reads window.casesPrefs once, when it loads, and keeps its
// state with preact/hooks, so this file sets up a stand-in page before it
// loads the module, and gives it the stand-in for preact/hooks that
// composer.test.js uses (hooks-stand-in.js). Both need a process of their
// own, as `node --test` gives each file by default.
//
// The page is globalThis itself (window === globalThis), so that deleting
// globalThis.Notification makes a browser with no notifications, as
// options.ts tells them apart with `"Notification" in window`.
import { test } from "node:test";
import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import { render } from "preact-render-to-string";

const standIn = new URL("./hooks-stand-in.js", import.meta.url).href;
registerHooks({
  resolve(specifier, context, next) {
    if (specifier === "preact/hooks" && context.parentURL?.endsWith("/static/options.ts")) return next(standIn, context);
    return next(specifier, context);
  },
});

// The kept choices, as theme.ts would keep them, logging what options.ts
// does to them in `log`.
const prefs = {
  values: {},
  log: [],
  options: [
    { name: "look", values: ["mono", "daylight", "keel"] },
    { name: "theme", values: ["system", "light", "dark"] },
    { name: "notify", values: ["off", "blocking", "all"] },
  ],
  read(name) {
    return this.values[name] ?? "";
  },
  write(name, value) {
    this.log.push(`write ${name}=${value}`);
    this.values[name] = value;
  },
  apply() {
    this.log.push("apply");
  },
};
globalThis.window = globalThis;
globalThis.casesPrefs = prefs;
// The window's focus, which a click on a notification asks for.
let focused = 0;
globalThis.focus = () => focused++;

// localStorage, which can be made to throw on reads, writes, or both. The
// writes made are in `writes`.
const stored = new Map();
const writes = [];
const storage = { readFails: false, writeFails: false };
Object.defineProperty(globalThis, "localStorage", {
  configurable: true,
  value: {
    getItem(k) {
      if (storage.readFails) throw new Error("SecurityError");
      return stored.has(k) ? stored.get(k) : null;
    },
    setItem(k, v) {
      if (storage.writeFails) throw new Error("QuotaExceededError");
      writes.push([k, String(v)]);
      stored.set(k, String(v));
    },
    removeItem(k) {
      if (storage.writeFails) throw new Error("SecurityError");
      stored.delete(k);
    },
  },
});

const { mount } = await import("./hooks-stand-in.js");
const { Options, Keys, notify } = await import("../static/options.ts");

// The browser's Notification, with permission `permission`. Asked, it
// answers `answer`. The notifications made are in `shown`; the constructor
// throws for a title `fails` says yes to.
function browser(permission, { answer = "granted", fails = () => false } = {}) {
  const shown = [];
  class Notification {
    static permission = permission;
    static asked = 0;
    static requestPermission() {
      Notification.asked++;
      return Promise.resolve(answer);
    }
    constructor(title, options) {
      if (fails(title)) throw new TypeError("Illegal constructor");
      Object.assign(this, { title, options, closed: false });
      shown.push(this);
    }
    close() {
      this.closed = true;
    }
  }
  Notification.shown = shown;
  globalThis.Notification = Notification;
  return Notification;
}
const unsupported = () => delete globalThis.Notification;

// Lets every promise that can settle settle.
async function settle() {
  for (let i = 0; i < 20; i++) await new Promise((r) => setImmediate(r));
}

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
const text = (v) => [...nodes(v)].flatMap((n) => [n.props?.children].flat().filter((c) => typeof c === "string" || typeof c === "number")).join("");

// The options dialog, drawn with the choices `values` kept.
function open(values = {}) {
  prefs.values = { ...values };
  prefs.log = [];
  const dialog = { current: null };
  const c = mount(Options, { dialog });
  const view = {
    dialog,
    get tree() {
      return c.tree;
    },
    page: () => render(c.tree),
    all: (pick) => [...nodes(c.tree)].filter(pick),
    find(pick, what = "a vnode") {
      const v = [...nodes(c.tree)].find(pick);
      assert.ok(v, `no ${what}: ${render(c.tree)}`);
      return v;
    },
    radio: (name, value) => view.find((v) => v.type === "input" && v.props.name === name && v.props.value === value, `radio ${name}=${value}`),
    // Each group's radios: its name, and its values with the checked one
    // marked.
    groups: () =>
      view.all((v) => v.type === "fieldset").map((f) => [
        text(f.props.children[0]),
        [...nodes(f)].filter((v) => v.type === "input").map((v) => `${v.props.name}=${v.props.value}${v.props.checked ? "*" : ""}`),
      ]),
    status: () => text(view.find((v) => v.props?.id === "notify-status", "status")),
    allow: () => view.find((v) => v.props?.id === "notify-allow", "allow button"),
    async act(fn) {
      fn();
      c.draw();
      await settle();
      c.draw();
    },
  };
  return view;
}

test("the options show each group, appearance then behaviour, with the kept choice checked", () => {
  browser("granted");
  const o = open({ look: "keel", theme: "dark", face: "serif", size: "large", wrap: "wrap", links: "same", notify: "blocking", hints: "hide", approve: "on" });
  assert.equal(o.tree.type, "dialog");
  assert.equal(o.tree.props.id, "options-dialog");
  assert.equal(o.tree.ref, o.dialog);
  assert.equal(text(o.find((v) => v.type === "h2")), "options");
  assert.equal(text(o.find((v) => v.type === "p" && v.props.class === "label")), "kept in this browser. changes apply at once.");
  assert.deepEqual(o.all((v) => v.type === "section").map((s) => [s.key, s.props["aria-labelledby"], text(s.props.children[0])]), [
    ["appearance", "options-appearance", "appearance"],
    ["behaviour", "options-behaviour", "behaviour"],
  ]);
  assert.deepEqual(o.groups(), [
    ["look", ["look=mono", "look=daylight", "look=keel*"]],
    ["theme", ["theme=system", "theme=light", "theme=dark*"]],
    ["prose face", ["face=mono", "face=sans", "face=serif*"]],
    ["text size", ["size=small", "size=medium", "size=large*"]],
    ["long lines in code", ["wrap=scroll", "wrap=wrap*"]],
    ["links to other sites", ["links=new", "links=same*"]],
    ["desktop notifications", ["notify=off", "notify=blocking*", "notify=all"]],
    ["key hints", ["hints=show", "hints=hide*"]],
    ["pre-select approve for approvals", ["approve=off", "approve=on*"]],
  ]);
  const page = o.page();
  for (const label of ["scroll sideways", "open in a new tab", "open in this tab", "blocking cases", "every case"]) assert.ok(page.includes(label), label);
  assert.ok(page.includes('<span class="face-sans">sans</span>'));
  assert.ok(page.includes('<span class="face-serif">serif</span>'));
  assert.ok(page.includes('<button type="button" id="options-reset">reset</button><button type="submit">close</button>'));
  assert.ok(page.includes('<form id="options" method="dialog">'));
});

test("only the first group's kept choice takes the focus when the dialog opens", () => {
  browser("granted");
  const o = open({ look: "daylight", theme: "dark" });
  const focused = o.all((v) => v.type === "input" && v.props.autofocus).map((v) => `${v.props.name}=${v.props.value}`);
  assert.deepEqual(focused, ["look=daylight"]);
  // With no look kept, nothing does.
  assert.deepEqual(open({ theme: "dark" }).all((v) => v.type === "input" && v.props.autofocus), []);
});

test("the notifications group says what the browser allows, with the allow button only while it would ask", () => {
  const cases = [
    ["granted", "allowed in this browser."],
    ["denied", "blocked in this browser. allow them in its site settings for this address."],
    ["default", "the browser asks before the first one."],
  ];
  for (const [permission, says] of cases) {
    browser(permission);
    const o = open();
    assert.equal(o.status(), says, permission);
    assert.equal(o.find((v) => v.props?.id === "notify-status").props.role, "status");
    assert.equal(o.allow().props.hidden, permission !== "default", permission);
    assert.equal(text(o.allow()), "allow notifications");
  }
  unsupported();
  const o = open();
  assert.equal(o.status(), "this browser cannot show them.");
  assert.equal(o.allow().props.hidden, true);
  // The status and button are in the notifications group, and no other.
  assert.equal(o.all((v) => v.props?.id === "notify-status").length, 1);
  const group = o.find((v) => v.type === "fieldset" && v.key === "notify");
  assert.ok([...nodes(group)].some((v) => v.props?.id === "notify-allow"));
});

test("a choice is kept and applied at once, and drawn checked", async () => {
  browser("default");
  const o = open({ theme: "system" });
  await o.act(() => o.radio("theme", "dark").props.onChange());
  assert.deepEqual(prefs.log, ["write theme=dark", "apply"]);
  assert.equal(o.radio("theme", "dark").props.checked, true);
  assert.equal(o.radio("theme", "system").props.checked, false);
  // Not a notifications choice, so the browser is not asked.
  assert.equal(Notification.asked, 0);
});

test("turning notifications on asks the browser only while it would ask", async () => {
  // Granted already, or blocked: nothing to ask.
  for (const permission of ["granted", "denied"]) {
    const N = browser(permission);
    const o = open({ notify: "off" });
    await o.act(() => o.radio("notify", "all").props.onChange());
    assert.equal(N.asked, 0, permission);
    assert.deepEqual(prefs.log, ["write notify=all", "apply"], permission);
  }
  // Turning them off does not ask.
  const N = browser("default");
  const o = open({ notify: "all" });
  await o.act(() => o.radio("notify", "off").props.onChange());
  assert.equal(N.asked, 0);
});

test("allowed when asked, notifications stay on; refused or dismissed, they go back off", async () => {
  for (const [answer, log] of [
    ["granted", ["write notify=blocking", "apply", "apply"]],
    ["denied", ["write notify=blocking", "apply", "write notify=off", "apply"]],
    ["default", ["write notify=blocking", "apply", "write notify=off", "apply"]],
  ]) {
    const N = browser("default", { answer });
    const o = open({ notify: "off" });
    await o.act(() => o.radio("notify", "blocking").props.onChange());
    assert.equal(N.asked, 1, answer);
    assert.deepEqual(prefs.log, log, answer);
    assert.equal(o.radio("notify", answer === "granted" ? "blocking" : "off").props.checked, true, answer);
  }
});

test("the allow button asks the browser, and a refusal turns notifications off", async () => {
  let N = browser("default", { answer: "denied" });
  let o = open({ notify: "all" });
  await o.act(() => o.allow().props.onClick());
  assert.equal(N.asked, 1);
  assert.deepEqual(prefs.log, ["write notify=off", "apply"]);
  N = browser("default", { answer: "granted" });
  o = open({ notify: "all" });
  await o.act(() => o.allow().props.onClick());
  assert.equal(N.asked, 1);
  assert.deepEqual(prefs.log, ["apply"]);
  assert.equal(o.radio("notify", "all").props.checked, true);
});

test("reset keeps each option's first value, applies them once, and draws them", async () => {
  browser("granted");
  const o = open({ look: "keel", theme: "dark", notify: "all" });
  await o.act(() => o.find((v) => v.props?.id === "options-reset").props.onClick());
  assert.deepEqual(prefs.log, ["write look=mono", "write theme=system", "write notify=off", "apply"]);
  assert.equal(o.radio("look", "mono").props.checked, true);
  assert.equal(o.radio("theme", "system").props.checked, true);
  assert.equal(o.radio("notify", "off").props.checked, true);
});

// A dialog as the backdrop handlers see it: closing it is counted.
function backdropOf(tree) {
  const el = {
    closed: 0,
    close() {
      this.closed++;
    },
  };
  const inside = {};
  const press = (target) => tree.props.onPointerDown({ target, currentTarget: el });
  const click = (target) => tree.props.onClick({ target, currentTarget: el });
  return { el, inside, press, click };
}

for (const [name, draw] of [
  ["options", () => open().tree],
  ["keys", () => Keys({ dialog: { current: null } })],
]) {
  test(`the ${name} dialog closes on a press and release on its backdrop, and not on a drag from inside`, () => {
    browser("granted");
    const d = backdropOf(draw());
    // Pressed and released on the backdrop: closes.
    d.press(d.el);
    d.click(d.el);
    assert.equal(d.el.closed, 1);
    // A click with no press first (the press was used up): stays.
    d.click(d.el);
    assert.equal(d.el.closed, 1);
    // Pressed inside the form, released on the backdrop: stays.
    d.press(d.inside);
    d.click(d.el);
    assert.equal(d.el.closed, 1);
    // Pressed on the backdrop, released inside: stays, and the press is
    // used up.
    d.press(d.el);
    d.click(d.inside);
    d.click(d.el);
    assert.equal(d.el.closed, 1);
  });
}

test("the keys dialog lists the keys, in a dialog the ref holds", () => {
  const dialog = { current: null };
  const tree = Keys({ dialog });
  assert.equal(tree.type, "dialog");
  assert.equal(tree.props.id, "keys-dialog");
  assert.equal(tree.props["aria-labelledby"], "keys-title");
  assert.equal(tree.ref, dialog);
  const page = render(tree);
  assert.ok(page.includes('<h2 class="section" id="keys-title">keys</h2>'));
  const says = [...nodes(tree)].filter((v) => v.type === "dd").map(text);
  assert.deepEqual(says, [
    "send the answer, even from a text field",
    "choose that option (decision, sign-off, stuck)",
    "approve, hold or don't run the current row of an approval, then go to the next row with none",
    "go to the note to the agent",
    "next or previous case in the list",
    "this list",
  ]);
  assert.ok(page.includes("only the send keys work while typing in a field."));
});

// The desktop notifications (notify).

const CURSOR = "cases.notify-cursor";
const kept = () => (stored.has(CURSOR) ? JSON.parse(stored.get(CURSOR)) : null);
const keep = (c) => stored.set(CURSOR, JSON.stringify(c));

const item = (id, urgency = "today") => ({
  id,
  tag: `case-${id}`,
  title: `title ${id}`,
  body: `body ${id}`,
  url: `/cases/c${id}`,
  case: urgency === null ? undefined : { urgency },
});

// A stand-in for serve's notifications feed: each GET takes the next of
// `answers` (a page, a function giving one or a Response, or an Error to
// throw), and the last stays. The paths asked are in `asked`.
function feed(answers) {
  const asked = [];
  globalThis.fetch = async (url) => {
    asked.push(url);
    const next = answers.length > 1 ? answers.shift() : answers[0];
    const answer = typeof next === "function" ? next() : next;
    if (answer instanceof Error) throw answer;
    if (answer instanceof Response) return answer;
    return new Response(JSON.stringify(answer), { status: 200 });
  };
  return asked;
}
const page = (boot, latest, items = []) => ({ boot, latest, items });

// Starts notify with a mock clock, and lets its first check finish. Clicks
// on a notification open the paths in `opened`. `tick(ms)` moves the clock
// on and lets what that starts finish. The loop ends with the test, as the
// mock clock drops what is pending; nothing is in flight by then.
async function start(t) {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const opened = [];
  notify((path) => opened.push(path));
  await settle();
  return {
    opened,
    async tick(ms) {
      t.mock.timers.tick(ms);
      await settle();
    },
  };
}

// Forgets the cursor, in localStorage and this tab's copy, as a check with
// notifications off does.
async function forget(t) {
  storage.readFails = storage.writeFails = false;
  browser("granted");
  prefs.values = { notify: "off" };
  feed([page("x", 0)]);
  await t.test("forget", async (t) => {
    await start(t);
  });
  stored.clear();
  prefs.values = {};
}

test("with notifications off, or not allowed, serve is not asked and the cursor is dropped", async (t) => {
  for (const [notifyPref, setup] of [
    ["off", () => browser("granted")],
    ["all", () => browser("default")],
    ["all", () => browser("denied")],
    ["blocking", unsupported],
  ]) {
    await forget(t);
    setup();
    prefs.values = { notify: notifyPref };
    keep({ boot: "b1", id: 4 });
    const asked = feed([page("b1", 9, [item(5)])]);
    await t.test(`${notifyPref}, ${globalThis.Notification?.permission ?? "unsupported"}`, async (t) => {
      const n = await start(t);
      assert.deepEqual(asked, []);
      assert.equal(stored.has(CURSOR), false);
      // And it keeps checking, at the same pace.
      await n.tick(4999);
      assert.deepEqual(asked, []);
      prefs.values = { notify: "all" };
      browser("granted");
      await n.tick(1);
      assert.deepEqual(asked, ["/v1/notifications"]);
    });
  }
});

test("a tab with no cursor starts at the latest and shows nothing, then asks after it every five seconds", async (t) => {
  await forget(t);
  const N = browser("granted");
  prefs.values = { notify: "all" };
  const asked = feed([page("b1", 7, [item(6), item(7)]), page("b1", 7)]);
  const n = await start(t);
  assert.deepEqual(asked, ["/v1/notifications"]);
  assert.deepEqual(N.shown, []);
  assert.deepEqual(kept(), { boot: "b1", id: 7 });
  await n.tick(4999);
  assert.equal(asked.length, 1);
  await n.tick(1);
  assert.deepEqual(asked, ["/v1/notifications", "/v1/notifications?after=7"]);
});

test("new notifications are shown once each, and a click opens the case", async (t) => {
  await forget(t);
  const N = browser("granted");
  prefs.values = { notify: "all" };
  keep({ boot: "b1", id: 3 });
  // The feed may hold items at or before the cursor: they are not shown.
  const asked = feed([page("b1", 5, [item(3), item(4), item(5, "blocking")]), page("b1", 5)]);
  const n = await start(t);
  assert.deepEqual(asked, ["/v1/notifications?after=3"]);
  assert.deepEqual(
    N.shown.map((s) => [s.title, s.options]),
    [
      ["title 4", { body: "body 4", tag: "case-4" }],
      ["title 5", { body: "body 5", tag: "case-5" }],
    ],
  );
  assert.deepEqual(kept(), { boot: "b1", id: 5 });
  const before = focused;
  N.shown[1].onclick();
  assert.equal(focused, before + 1);
  assert.deepEqual(n.opened, ["/cases/c5"]);
  assert.equal(N.shown[1].closed, true);
  assert.equal(N.shown[0].closed, false);
  // The next check asks after the new cursor, and shows nothing again.
  await n.tick(5000);
  assert.deepEqual(asked, ["/v1/notifications?after=3", "/v1/notifications?after=5"]);
  assert.equal(N.shown.length, 2);
});

test("on for blocking cases only, only blocking cases are shown, and the cursor moves past the rest", async (t) => {
  await forget(t);
  const N = browser("granted");
  prefs.values = { notify: "blocking" };
  keep({ boot: "b1", id: 0 });
  feed([page("b1", 4, [item(1, "today"), item(2, "blocking"), item(3, null), item(4, "soon")])]);
  await start(t);
  assert.deepEqual(N.shown.map((s) => s.title), ["title 2"]);
  assert.deepEqual(kept(), { boot: "b1", id: 4 });
});

test("at or past the latest, nothing is shown and the cursor stays", async (t) => {
  await forget(t);
  const N = browser("granted");
  prefs.values = { notify: "all" };
  for (const id of [5, 9]) {
    keep({ boot: "b1", id });
    await t.test(`cursor ${id}`, async (t) => {
      feed([page("b1", 5, [item(5)])]);
      writes.length = 0;
      await start(t);
      assert.deepEqual(N.shown, []);
      assert.deepEqual(kept(), { boot: "b1", id });
      // Not even written again as it was.
      assert.deepEqual(writes, []);
    });
  }
});

test("after serve restarts, the cursor starts again from 0 under the new boot, and asks again at once", async (t) => {
  await forget(t);
  const N = browser("granted");
  prefs.values = { notify: "all" };
  keep({ boot: "b1", id: 40 });
  const asked = feed([page("b2", 2, [item(1), item(2)]), page("b2", 2, [item(1), item(2)])]);
  const n = await start(t);
  assert.deepEqual(N.shown, []);
  assert.deepEqual(kept(), { boot: "b2", id: 0 });
  await n.tick(0);
  assert.deepEqual(asked, ["/v1/notifications?after=40", "/v1/notifications?after=0"]);
  assert.deepEqual(N.shown.map((s) => s.title), ["title 1", "title 2"]);
  assert.deepEqual(kept(), { boot: "b2", id: 2 });
});

test("when another tab moves the cursor back while this one waits, it asks again at once", async (t) => {
  for (const [name, moved] of [
    ["to a new boot", { boot: "b2", id: 0 }],
    ["to an earlier number", { boot: "b1", id: 2 }],
  ]) {
    await forget(t);
    const N = browser("granted");
    prefs.values = { notify: "all" };
    keep({ boot: "b1", id: 6 });
    const asked = feed([
      () => {
        keep(moved);
        return page(moved.boot, 8, [item(7), item(8)]);
      },
      page(moved.boot, 8, [item(7), item(8)]),
    ]);
    await t.test(name, async (t) => {
      const n = await start(t);
      assert.deepEqual(N.shown, []);
      assert.deepEqual(kept(), moved);
      await n.tick(0);
      assert.deepEqual(asked, ["/v1/notifications?after=6", `/v1/notifications?after=${moved.id}`]);
      assert.deepEqual(N.shown.map((s) => s.title), ["title 7", "title 8"]);
    });
  }
});

test("when another tab moves the cursor on while this one waits, only what is after it is shown", async (t) => {
  await forget(t);
  const N = browser("granted");
  prefs.values = { notify: "all" };
  keep({ boot: "b1", id: 3 });
  const asked = feed([
    () => {
      keep({ boot: "b1", id: 5 });
      return page("b1", 7, [item(4), item(5), item(6), item(7)]);
    },
  ]);
  const n = await start(t);
  assert.deepEqual(N.shown.map((s) => s.title), ["title 6", "title 7"]);
  assert.deepEqual(kept(), { boot: "b1", id: 7 });
  // Not asked again at once.
  await n.tick(0);
  assert.equal(asked.length, 1);
});

test("a refusal, or serve out of reach, shows nothing and asks again at the next check", async (t) => {
  for (const [name, answer] of [
    ["refused", () => new Response(JSON.stringify({ error: { category: "internal", message: "no" } }), { status: 500 })],
    ["out of reach", () => new TypeError("Failed to fetch")],
  ]) {
    await forget(t);
    const N = browser("granted");
    prefs.values = { notify: "all" };
    keep({ boot: "b1", id: 3 });
    const asked = feed([answer, page("b1", 4, [item(4)])]);
    await t.test(name, async (t) => {
      const n = await start(t);
      assert.deepEqual(N.shown, []);
      assert.deepEqual(kept(), { boot: "b1", id: 3 });
      await n.tick(4999);
      assert.equal(asked.length, 1);
      await n.tick(1);
      assert.deepEqual(asked, ["/v1/notifications?after=3", "/v1/notifications?after=3"]);
      assert.deepEqual(N.shown.map((s) => s.title), ["title 4"]);
    });
  }
});

test("a kept cursor that is not one is ignored, and the tab starts at the latest", async (t) => {
  for (const junk of ["{not json", JSON.stringify({ boot: 1, id: 2 }), JSON.stringify({ boot: "b1", id: "2" }), JSON.stringify({ boot: "b1" }), "null"]) {
    await forget(t);
    const N = browser("granted");
    prefs.values = { notify: "all" };
    stored.set(CURSOR, junk);
    const asked = feed([page("b1", 9, [item(9)])]);
    await t.test(junk, async (t) => {
      await start(t);
      assert.deepEqual(asked, ["/v1/notifications"]);
      assert.deepEqual(N.shown, []);
      assert.deepEqual(kept(), { boot: "b1", id: 9 });
    });
  }
});

test("with localStorage out of use, the tab's own copy of the cursor stands in", async (t) => {
  await forget(t);
  const N = browser("granted");
  prefs.values = { notify: "all" };
  storage.readFails = storage.writeFails = true;
  const asked = feed([page("b1", 3, [item(3)]), page("b1", 5, [item(4), item(5)])]);
  const n = await start(t);
  assert.deepEqual(N.shown, []);
  await n.tick(5000);
  assert.deepEqual(asked, ["/v1/notifications", "/v1/notifications?after=3"]);
  assert.deepEqual(N.shown.map((s) => s.title), ["title 4", "title 5"]);
  assert.equal(stored.has(CURSOR), false);
  // Turned off, the tab's copy is dropped too, even though localStorage
  // cannot be written.
  prefs.values = { notify: "off" };
  await n.tick(5000);
  prefs.values = { notify: "all" };
  await n.tick(5000);
  assert.deepEqual(asked.slice(2), ["/v1/notifications"]);
  storage.readFails = storage.writeFails = false;
});

test("with localStorage readable but not writable, the tab's copy is used once storage has none", async (t) => {
  await forget(t);
  browser("granted");
  prefs.values = { notify: "all" };
  storage.writeFails = true;
  const asked = feed([page("b1", 3), page("b1", 3)]);
  const n = await start(t);
  assert.equal(stored.has(CURSOR), false);
  await n.tick(5000);
  assert.deepEqual(asked, ["/v1/notifications", "/v1/notifications?after=3"]);
  storage.writeFails = false;
});

// Pinned as it is: a cursor kept in localStorage wins over this tab's copy,
// so when storage still reads but stops taking writes (its quota full), the
// tab asks after the stale number at every check and shows the same
// notifications again, every five seconds. Turning notifications off cannot
// drop it either, so turning them on again replays what came in meanwhile,
// which the comment on notify says must not happen. Reported as a
// follow-up; not changed here.
test("with an old cursor kept and localStorage no longer taking writes, the same notifications are shown again", async (t) => {
  await forget(t);
  const N = browser("granted");
  prefs.values = { notify: "all" };
  keep({ boot: "b1", id: 3 });
  storage.writeFails = true;
  t.after(() => (storage.writeFails = false));
  const asked = feed([page("b1", 5, [item(4), item(5)])]);
  const n = await start(t);
  await n.tick(5000);
  assert.deepEqual(asked, ["/v1/notifications?after=3", "/v1/notifications?after=3"]);
  assert.deepEqual(N.shown.map((s) => s.title), ["title 4", "title 5", "title 4", "title 5"]);
  // Off and on again: the stale cursor is still there.
  prefs.values = { notify: "off" };
  await n.tick(5000);
  assert.deepEqual(kept(), { boot: "b1", id: 3 });
  prefs.values = { notify: "all" };
  await n.tick(5000);
  assert.equal(asked.at(-1), "/v1/notifications?after=3");
  assert.equal(N.shown.length, 6);
});

// A Notification the browser will not make (Chrome on Android throws
// "Illegal constructor" from `new Notification`) is passed over, and the
// cursor moves past it all the same: each item is tried once, shown or not,
// so the items before it are not shown again, and the items after it are
// tried.
test("a notification the browser will not make is passed over, and the others are shown once", async (t) => {
  await forget(t);
  const N = browser("granted", { fails: (title) => title === "title 2" });
  prefs.values = { notify: "all" };
  keep({ boot: "b1", id: 0 });
  const asked = feed([page("b1", 3, [item(1), item(2), item(3)]), page("b1", 3)]);
  const n = await start(t);
  assert.deepEqual(N.shown.map((s) => s.title), ["title 1", "title 3"]);
  assert.deepEqual(kept(), { boot: "b1", id: 3 });
  await n.tick(4999);
  assert.equal(asked.length, 1);
  await n.tick(1);
  assert.deepEqual(asked, ["/v1/notifications?after=0", "/v1/notifications?after=3"]);
  assert.equal(N.shown.length, 2);
});

test("in a browser that will make none, the cursor still moves on, and the tab asks at its usual pace", async (t) => {
  await forget(t);
  const N = browser("granted", { fails: () => true });
  prefs.values = { notify: "all" };
  keep({ boot: "b1", id: 0 });
  const asked = feed([page("b1", 2, [item(1), item(2)]), page("b1", 4, [item(1), item(2), item(3), item(4)])]);
  const n = await start(t);
  assert.deepEqual(kept(), { boot: "b1", id: 2 });
  await n.tick(0);
  assert.equal(asked.length, 1);
  await n.tick(5000);
  assert.deepEqual(asked, ["/v1/notifications?after=0", "/v1/notifications?after=2"]);
  assert.deepEqual(kept(), { boot: "b1", id: 4 });
  assert.deepEqual(N.shown, []);
});
