// What a row's withdraw (RowWithdraw in static/lists.ts, ST-14) does once it
// is pressed: render-to-string runs neither effects nor handlers, so these
// tests give lists.ts a stand-in for preact/hooks (hooks-stand-in.js), take
// RowWithdraw from the tree the inbox-zero panel gives, call it as a
// function, hand its refs stand-in elements that log their focus, and call
// the handlers in the tree it gives, with serve stood in for by a fetch
// that holds each POST until the test answers it.
//
// The stand-in is in place only if this file loads lists.ts first, so it
// needs a process of its own, as `node --test` gives each file by default.
import { test, mock } from "node:test";
import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import { render } from "preact-render-to-string";

const standIn = new URL("./hooks-stand-in.js", import.meta.url).href;
registerHooks({
  resolve(specifier, context, next) {
    if (specifier === "preact/hooks" && context.parentURL?.endsWith("/static/lists.ts")) return next(standIn, context);
    return next(specifier, context);
  },
});
const { mount } = await import("./hooks-stand-in.js");
const { List, Zero } = await import("../static/lists.ts");
const { decision, steps } = await import("./case.js");

const now = Date.parse("2026-09-25T13:40:00Z");

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

// The component called `name` that `tree` draws, mounting each component
// on the way down until it is found: its function and its props.
function find(tree, name) {
  for (const v of nodes(tree)) {
    if (typeof v.type !== "function") continue;
    if (v.type.name === name) return v;
    const inner = find(mount(v.type, v.props).tree, name);
    if (inner) return inner;
  }
  return null;
}

const RowWithdraw = find(Zero({ z: { today: 0, week: 0, closed: 0, lastAnswer: "", outbox: [], flights: [{ c: decision("answered", steps.open, steps.answer), who: "w", last: "answered", at: steps.answer.at }] }, now }), "RowWithdraw").type;

// The text of a vnode's children, where they are strings.
const text = (v) => [...nodes(v)].flatMap((n) => [n.props?.children].flat().filter((c) => typeof c === "string" || typeof c === "number")).join("");

// Stands in for serve: the token at /v1, and each POST held until the test
// answers it with `answer(i, status, body)`, or fails it with `lose(i)`.
function serve(t) {
  const was = globalThis.fetch;
  t.after(() => {
    globalThis.fetch = was;
  });
  const posts = [];
  globalThis.fetch = (url, init = {}) => {
    if (url === "/v1") return Promise.resolve(new Response(JSON.stringify({ csrf: "tok" }), { status: 200 }));
    return new Promise((resolve, reject) => posts.push({ url, body: JSON.parse(init.body), resolve, reject }));
  };
  const answer = (i, status, body) => posts[i].resolve(new Response(JSON.stringify(body), { status }));
  const lose = (i) => posts[i].reject(new TypeError("Failed to fetch"));
  return { posts, answer, lose };
}

const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setImmediate(r));
};

// A row's withdraw on case `c`, mounted: its tree, the focus its refs were
// given, what `sent` was called with, and the handlers of what it draws.
function row(c) {
  const log = [];
  const sent = [];
  const props = { c, sent: (what, after) => sent.push([what, after.revision]) };
  const attach = (tree) => {
    for (const v of nodes(tree)) {
      if (v.ref && v.type === "input") v.ref.current = { focus: () => log.push(`focus ${v.props.name}`) };
      if (v.ref && v.type === "button") v.ref.current = { focus: () => log.push("focus withdraw") };
    }
  };
  const m = mount(RowWithdraw, props, attach);
  const one = (pred) => [...nodes(m.tree)].find(pred);
  const r = {
    log,
    sent,
    props,
    get tree() {
      return m.tree;
    },
    html: () => render(m.tree),
    draw: () => m.draw(),
    // The stream brings `next`: the row is drawn again with it.
    bring(next) {
      props.c = next;
      m.dirty = true;
      m.draw();
    },
    press: () => {
      one((v) => v.type === "button" && v.props.class === "row-withdraw").props.onClick();
      m.draw();
    },
    type: (value) => {
      one((v) => v.type === "input").props.onInput({ target: { value } });
      m.draw();
    },
    keep: () => {
      one((v) => v.type === "button" && text(v) === "keep").props.onClick();
      m.draw();
    },
    key: (key) => {
      one((v) => v.type === "form").props.onKeyDown({ key });
      m.draw();
    },
    // Submits the form; `prevented` says whether its default was prevented.
    submit() {
      const e = { prevented: false, preventDefault() { this.prevented = true; } };
      const done = one((v) => v.type === "form").props.onSubmit(e);
      return { e, done };
    },
    refusal: () => text(one((v) => v.type === "p" && v.props.class === "error")),
  };
  return r;
}

const answered = decision("answered", steps.open, steps.answer);
const pickedUp = decision("pickedup", steps.open, steps.answer, steps.pickup);

test("a row's withdraw is a button named for its case, which opens the line with the focus in the reason", () => {
  const r = row(answered);
  assert.equal(r.html(), `<button type="button" class="row-withdraw" aria-label="withdraw ${answered.title}">withdraw</button>`);
  assert.deepEqual(r.log, [], "nothing is focused before it is pressed");
  r.press();
  assert.equal(
    r.html(),
    '<form class="row-confirm" data-write="withdraw"><label>reason, if any <input type="text" name="reason" value/></label><div class="actions"><button type="submit">withdraw it</button><button type="button">keep</button></div></form>',
  );
  assert.deepEqual(r.log, ["focus reason"]);
});

test("keep, or Escape, puts the button back with the focus on it, the reason and any refusal gone", async (t) => {
  const { posts, answer } = serve(t);
  for (const close of [(r) => r.keep(), (r) => r.key("Escape")]) {
    const r = row(answered);
    r.press();
    r.type("not needed");
    const { done } = r.submit();
    await settle();
    answer(posts.length - 1, 403, { error: { category: "request", message: "the case is not yours to withdraw" } });
    await done;
    r.draw();
    assert.equal(r.refusal(), "the case is not yours to withdraw");
    close(r);
    assert.match(r.html(), /^<button type="button" class="row-withdraw"/);
    assert.deepEqual(r.log, ["focus reason", "focus withdraw"]);
    r.press();
    assert.match(r.html(), /name="reason" value\/>/, "the reason is gone");
    assert.equal(r.refusal(), "", "the refusal is gone");
  }
});

test("another key in the line does nothing", () => {
  const r = row(answered);
  r.press();
  r.key("Enter");
  r.key("a");
  assert.match(r.html(), /^<form class="row-confirm"/);
});

test("the withdraw is sent at the revision the row had when it was pressed, with the reason trimmed, or none", async (t) => {
  const { posts, answer } = serve(t);
  const r = row(answered);
  r.press();
  r.type("  not needed  ");
  const { e, done } = r.submit();
  assert.equal(e.prevented, true, "the form is not submitted by the browser");
  await settle();
  assert.deepEqual(posts.map((p) => [p.url, p.body]), [[`/v1/cases/${answered.id}/withdraw`, { revision: answered.revision, reason: "not needed" }]]);
  answer(0, 200, { ...answered, state: "withdrawn", revision: answered.revision + 1 });
  await done;
  assert.deepEqual(r.sent, [["withdraw", answered.revision + 1]], "the page is told what landed");
  const blank = row(answered);
  blank.press();
  blank.type("   ");
  const second = blank.submit();
  await settle();
  assert.deepEqual(posts[1].body, { revision: answered.revision }, "a blank reason is no reason");
  answer(1, 200, { ...answered, state: "withdrawn", revision: answered.revision + 1 });
  await second.done;
});

test("a withdraw that lands with no one told of it leaves the line as it was", async (t) => {
  const { answer } = serve(t);
  const r = row(answered);
  r.props.sent = undefined;
  r.draw();
  r.press();
  const { done } = r.submit();
  await settle();
  answer(0, 200, { ...answered, state: "withdrawn", revision: answered.revision + 1 });
  await done;
  r.draw();
  assert.match(r.html(), /^<form class="row-confirm"/, "the stream takes the row away, not the row itself");
  assert.equal(r.refusal(), "");
});

test("a refusal is said in the line, as serve says it, and the line stays open to send again", async (t) => {
  const { posts, answer } = serve(t);
  const refusals = [
    [403, { error: { category: "request", message: "a write from another site" } }, "a write from another site"],
    [409, { error: { category: "transition", message: "the case is closed" } }, "the case is closed"],
    [500, null, "serve answered 500"],
  ];
  for (const [status, body, said] of refusals) {
    const r = row(answered);
    r.press();
    const { done } = r.submit();
    await settle();
    answer(posts.length - 1, status, body);
    await done;
    r.draw();
    assert.equal(r.refusal(), said);
    assert.match(r.html(), /<p class="error" role="alert">/);
    assert.deepEqual(r.sent, []);
    // It can be sent again, at the same revision.
    const again = r.submit();
    await settle();
    assert.deepEqual(posts[posts.length - 1].body, { revision: answered.revision });
    answer(posts.length - 1, status, body);
    await again.done;
  }
});

test("a stale refusal says so, and the line takes the case the refusal shows, to be sent at once it is seen", async (t) => {
  const { posts, answer } = serve(t);
  const r = row(answered);
  r.press();
  const first = r.submit();
  await settle();
  // Refused as stale: the case was picked up since the row was drawn.
  answer(0, 409, { error: { category: "stale", message: "stale", case: pickedUp } });
  await first.done;
  r.draw();
  // The line takes the refusal's case against the case the row was drawn
  // with, which is older, so nothing more is said than the stale refusal.
  assert.equal(r.refusal(), "this case changed after it was drawn, so this was not recorded. check the thread before sending again.");
  const second = r.submit();
  await settle();
  assert.deepEqual(posts[1].body, { revision: pickedUp.revision }, "sent again at the case the refusal showed");
  answer(1, 200, { ...pickedUp, state: "withdrawn", revision: pickedUp.revision + 1 });
  await second.done;
  assert.deepEqual(r.sent, [["withdraw", pickedUp.revision + 1]]);
});

test("a stale refusal whose case is in the state the row was drawn in says the stale message, and takes its revision", async (t) => {
  const { posts, answer } = serve(t);
  const later = { ...answered, revision: answered.revision + 1 };
  const r = row(answered);
  r.press();
  const first = r.submit();
  await settle();
  answer(0, 409, { error: { category: "stale", message: "stale", case: later } });
  await first.done;
  r.draw();
  assert.equal(r.refusal(), "this case changed after it was drawn, so this was not recorded. check the thread before sending again.");
  const second = r.submit();
  await settle();
  assert.equal(posts[1].body.revision, later.revision);
  answer(1, 409, { error: { category: "stale", message: "stale" } });
  await second.done;
});

test("a stale refusal of a case older than one the stream brought while the send was out puts the line back on the older one", async (t) => {
  const { posts, answer } = serve(t);
  const r = row(answered);
  r.press();
  const first = r.submit();
  await settle();
  // The stream brings the case picked up while the send is out.
  r.bring(pickedUp);
  assert.equal(r.refusal(), "this case was picked up since you pressed withdraw. withdraw it if you still mean to.");
  // The refusal shows the case as it was before the pickup: late.
  answer(0, 409, { error: { category: "stale", message: "stale", case: answered } });
  await first.done;
  r.draw();
  assert.equal(r.refusal(), "this case changed after it was drawn, so this was not recorded. check the thread before sending again.");
  const second = r.submit();
  await settle();
  // Pinned as it is today, and reported: submit compares the refusal's case
  // with the case of the draw it was sent from, not the newer one the
  // stream brought while it was out, so the line goes back to the older
  // revision, and the next send is refused as stale once more. Nothing is
  // recorded that the human did not see.
  assert.equal(posts[1].body.revision, answered.revision);
  answer(1, 409, { error: { category: "stale", message: "stale" } });
  await second.done;
});

test("serve out of reach says the outcome is not known, and the line stays open", async (t) => {
  const { lose } = serve(t);
  const r = row(answered);
  r.press();
  const { done } = r.submit();
  await settle();
  lose(0);
  await done;
  r.draw();
  assert.equal(r.refusal(), "serve could not be reached, so this may or may not have been recorded. check the thread before sending again.");
  assert.deepEqual(r.sent, []);
});

test("while the line is open it follows the stream; while it is shut, a case that moved on is taken when it is pressed", async (t) => {
  const { posts, answer } = serve(t);
  const r = row(answered);
  // Shut: the stream's case is not followed, but is taken on the press.
  r.bring(pickedUp);
  assert.match(r.html(), /^<button type="button" class="row-withdraw"/, "shut, it stays the button");
  r.press();
  assert.equal(r.refusal(), "", "nothing to say: the line is drawn on the case as it is");
  const { done } = r.submit();
  await settle();
  assert.equal(posts[0].body.revision, pickedUp.revision);
  answer(0, 200, { ...pickedUp, state: "withdrawn", revision: pickedUp.revision + 1 });
  await done;
  // Open: the line follows, and says a state the case moved on to.
  const o = row(answered);
  o.press();
  o.bring({ ...answered, revision: answered.revision });
  assert.equal(o.refusal(), "", "the same revision says nothing");
  o.bring(pickedUp);
  assert.equal(o.refusal(), "this case was picked up since you pressed withdraw. withdraw it if you still mean to.");
});

test("a second submit while the first send is out sends nothing", async (t) => {
  const { posts, answer } = serve(t);
  const r = row(answered);
  r.press();
  const first = r.submit();
  const second = r.submit();
  await settle();
  assert.equal(posts.length, 1);
  await second.done;
  answer(0, 409, { error: { category: "transition", message: "no" } });
  await first.done;
  // Answered, the guard is released: the next submit is sent.
  const third = r.submit();
  await settle();
  assert.equal(posts.length, 2);
  answer(1, 409, { error: { category: "transition", message: "no" } });
  await third.done;
});

test("a send out for more than 10 s lets the next go, and its late answer lets a third go while the second is out", async (t) => {
  mock.timers.enable({ apis: ["setTimeout"] });
  t.after(() => mock.timers.reset());
  const { posts, answer } = serve(t);
  const r = row(answered);
  r.press();
  const a = r.submit();
  await settle();
  r.submit();
  await settle();
  assert.equal(posts.length, 1, "within 10 s a second submit sends nothing");
  mock.timers.tick(10000);
  const b = r.submit();
  await settle();
  assert.equal(posts.length, 2, "after 10 s the guard lets a second withdraw go");
  // The first answers while the second is out. Pinned as it is today: its
  // release frees the guard the second holds, so a third is sent while the
  // second is still out (the case view's sendOnce does not; reported). Each
  // carries the same revision, so serve records at most one.
  answer(0, 409, { error: { category: "transition", message: "no" } });
  await a.done;
  const c = r.submit();
  await settle();
  assert.equal(posts.length, 3);
  assert.deepEqual(posts.map((p) => p.body.revision), [answered.revision, answered.revision, answered.revision]);
  answer(1, 409, { error: { category: "transition", message: "no" } });
  answer(2, 409, { error: { category: "transition", message: "no" } });
  await b.done;
  await c.done;
});

// A part of the list folded by the human, as List draws it under the
// stand-in hooks.
test("a folded part of the list stays as the human left it, and opens when it comes to hold the case shown", () => {
  const briefing = { ...answered, id: "b1", kind: "briefing", answer: { answered_at: steps.answer.at } };
  const props = { inbox: [], done: [], briefings: [briefing], show: "briefings", selected: "", now };
  const folded = find(mount(List, props).tree, "Folded");
  assert.ok(folded, "the answered briefings are a folded part");
  const m = mount(folded.type, folded.props);
  const details = () => [...nodes(m.tree)].find((v) => v.type === "details");
  assert.equal(details().props.open, undefined, "folded at first, not holding the case shown");
  details().props.onToggle({ target: { open: true } });
  m.draw();
  assert.equal(details().props.open, true, "opened by the human");
  details().props.onToggle({ target: { open: false } });
  m.draw();
  assert.equal(details().props.open, undefined, "and folded again");
  const holding = mount(folded.type, { ...folded.props, holds: true });
  assert.equal([...nodes(holding.tree)].find((v) => v.type === "details").props.open, true, "open when it holds the case shown");
});
