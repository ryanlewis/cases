import { test } from "node:test";
import assert from "node:assert/strict";
import { render } from "preact-render-to-string";
import { html } from "../static/html.ts";
import { Composer, Entry, RECENT, drafted, filtered, kept, onKey, opening, picks, title } from "../static/compose.ts";

const now = Date.parse("2026-09-27T12:00:00Z");
const at = (minutes) => new Date(now - minutes * 60000).toISOString();
const listener = (worker, labels, extra = {}) => ({ worker, labels, waits: [{ labels, kinds: [] }], since: at(90), heard: at(0.1), ...extra });
const kase = (id, worker, labels, minutes, kind = "decision") => ({ id, kind, state: "open", urgency: "today", title: `case ${id}`, worker, labels, updated_at: at(minutes), events: [] });

test("the picker lists the agents listening now, then the workers and labels of recent cases that no one listens for", () => {
  const listeners = [listener("lead", ["release"]), listener("bun-pins", ["release", "deps"])];
  const cases = [
    kase("a1", "docs", ["docs"], 60),
    kase("a2", "lead", ["ops"], 60),
    kase("a3", "old", ["old"], RECENT / 60000 + 1),
    kase("a4", "writer", ["notes"], 5, "message"),
  ];
  const keys = picks(listeners, cases, now).map((p) => `${p.key}${p.live ? "" : " (not listening)"}`);
  // A label someone listens on is not offered: it is sent to as another.
  assert.deepEqual(keys, [
    "worker:bun-pins",
    "worker:lead",
    "worker:docs (not listening)",
    "label:docs (not listening)",
    "label:ops (not listening)",
  ]);
  assert.ok(!picks(listeners, [kase("a5", "docs", ["release"], 5)], now).some((p) => p.key === "label:release"));
  // Two days back, and no further.
  assert.equal(RECENT, 2 * 24 * 60 * 60 * 1000);
  assert.deepEqual(picks([], [kase("b1", "fresh", [], 47 * 60), kase("b2", "stale", [], 49 * 60)], now).map((p) => p.key), ["worker:fresh"]);
});

const fields = (extra) => ({ pick: "worker:bun-pins", other: "", otherKind: "worker", body: " Rebase onto main\n\nThen test. ", ...extra });

// The same examples as title_of's tests in message.rs, so the two stay one
// rule.
test("a message's title is the first line of its note without its markdown", () => {
  for (const [note, want] of [
    ["Rebase onto main\n\nThen run the tests.", "Rebase onto main"],
    ["## Rebase onto *main*\nThen test.", "Rebase onto main"],
    ["> - 1. Pin `bun` to **1.2**", "Pin bun to 1.2"],
    ["3) ship it", "ship it"],
    ["-flag stays", "-flag stays"],
    ["#123 is fixed", "#123 is fixed"],
    ["```sh\nrm -rf target\n```", "rm -rf target"],
    ["> ~~~\nship it", "ship it"],
    ["\u{feff}# Rebase", "Rebase"],
    ["Rebase", "Rebase"],
    ["\n  \n\tRebase  onto   main  \n", "Rebase onto main"],
    ["---\n#\n> \nRebase", "Rebase"],
    ["---\n# \n**", ""],
    ["", ""],
    ["Rebase the pinning branch onto main, then run the whole test suite and the mutants twice over", "Rebase the pinning branch onto main, then run the whole test suite and the…"],
    ["x".repeat(100), `${"x".repeat(80)}…`],
    ["x".repeat(80), "x".repeat(80)],
  ]) {
    assert.equal(title(note), want, JSON.stringify(note));
  }
});

test("a drafted message goes to the pick, with its text trimmed, and says what is missing", () => {
  const choices = picks([listener("bun-pins", ["release"])], [], now);
  assert.deepEqual(drafted(fields(), choices, "UkLWZ").body, { kind: "message", urgency: "today", title: "Rebase onto main", body: "Rebase onto main\n\nThen test.", worker: "bun-pins", re: "UkLWZ" });
  assert.deepEqual(drafted(fields({ pick: "label:release", body: "Rebase" }), kept(choices, "label:release"), "").body, { kind: "message", urgency: "today", title: "Rebase", body: "Rebase", labels: ["release"] });
  assert.deepEqual(drafted(fields({ pick: "other", other: " ops ", otherKind: "label" }), choices, "").body.labels, ["ops"]);
  assert.equal(drafted(fields({ pick: "" }), choices, "").problem, "choose who it goes to");
  assert.equal(drafted(fields({ pick: "other", other: " " }), choices, "").problem, "name the worker or label it goes to");
  assert.equal(drafted(fields({ body: " \n " }), choices, "").problem, "write a note");
  assert.equal(drafted(fields({ body: "---" }), choices, "").problem, "write a note");
});

const draw = (props) => render(html`<${Composer} to="" re="" listeners=${[]} cases=${[]} now=${now} onSent=${() => {}} onClose=${() => {}} ...${props} />`);

test("opened for no one, the composer offers the agents listening first, with what each is doing, and a quiet one dimmed", () => {
  const page = draw({ listeners: [listener("bun-pins", ["release"], { doing: "pinning bun", doing_at: at(3) }), listener("lead", [], { heard: at(0.5) })], cases: [kase("a1", "docs", [], 30)] });
  const at_ = (text) => {
    const i = page.indexOf(text);
    assert.notEqual(i, -1, `the composer does not say ${JSON.stringify(text)}`);
    return i;
  };
  assert.ok(at_("listening now") < at_("<strong>bun-pins</strong>"));
  assert.ok(at_("<strong>bun-pins</strong>") < at_("doing: pinning bun · 3m ago"));
  assert.ok(at_("<strong>lead</strong>") < at_("not heard from for 30s"));
  assert.ok(page.includes('<label class="choice pick quiet">'), page);
  // Those not listening are folded behind one line that counts them.
  assert.ok(at_("<strong>lead</strong>") < at_('aria-expanded="false">1 more, not listening</button>'), page);
  assert.ok(!page.includes("<strong>docs</strong>") && !page.includes('name="filter"'), page);
  // Agents alone: no row for a label, and no labels on an agent's row.
  assert.ok(!page.includes('<span class="tag">release</span>'), page);
  assert.ok(page.includes('<span>another</span>'), page);
  // One note, which has the focus, no title to write, and no how soon.
  assert.ok(!page.includes('name="title"') && /<textarea name="body" rows="5" autofocus/.test(page), page);
  assert.ok(!page.includes('name="urgency"') && !page.includes("how soon"), page);
  // It is a dialog, over the page.
  assert.ok(page.startsWith('<dialog class="options compose-dialog" id="compose-dialog"'), page);
  // A refusal is said just above the send button, where the eye is after
  // pressing it, below every field.
  assert.ok(at_('name="body"') < at_('<p class="error" role="alert" hidden>') && at_('<p class="error" role="alert" hidden>') < at_('<button type="submit"'), page);
});

test("opened for someone, the composer says who, with a change button and the case it is about, and shows no list", () => {
  const cases = [{ ...kase("UkLWZ", "ghost", [], 5), title: "Pin bun?" }];
  const page = draw({ to: "ghost", re: "UkLWZ", listeners: [listener("bun-pins", [])], cases });
  assert.ok(page.includes('<p class="to">to <strong>ghost</strong> <button type="button" class="change">change</button></p>'), page);
  assert.ok(page.includes('<p class="re">about <a href="/cases/UkLWZ-pin-bun">Pin bun?</a></p>'), page);
  assert.ok(!page.includes('name="to"') && !page.includes("listening now"), page);
  assert.ok(/<textarea name="body" rows="5" autofocus/.test(page) && !page.includes('name="urgency"'), page);
  // Opened for no one, it shows the picker.
  assert.ok(draw({ to: "", re: "" }).includes('<fieldset class="picker">'));
});

test("a composer opened for a worker picks it: listed, the worker of the case replied to, or another by name", () => {
  const offered = picks([listener("bun-pins", [])], [kase("a1", "ghost", [], 5)], now);
  assert.deepEqual(opening("bun-pins", undefined, offered), { pick: "worker:bun-pins", other: "" });
  // A worker of a recent case is listed, though not listening.
  assert.deepEqual(opening("ghost", undefined, offered), { pick: "worker:ghost", other: "" });
  // A reply about a case older than the picker looks back still goes to
  // its worker.
  const old = kase("b1", "gone", [], RECENT / 60000 + 60);
  assert.deepEqual(opening("gone", old, offered), { pick: "worker:gone", other: "" });
  assert.deepEqual(opening("nobody-yet", undefined, offered), { pick: "other", other: "nobody-yet" });
  assert.deepEqual(opening("", undefined, offered), { pick: "", other: "" });
  // It sends to that worker, today.
  const f = { ...opening("gone", old, offered), otherKind: "worker", body: "Rebase" };
  assert.deepEqual(drafted(f, kept(offered, f.pick), "b1").body, { kind: "message", urgency: "today", title: "Rebase", body: "Rebase", worker: "gone", re: "b1" });
});

test("an agent listening says since when, and not its labels", () => {
  const page = draw({ listeners: [listener("bun-pins", ["release", "deps"])] });
  const choice = page.slice(page.indexOf("<strong>bun-pins</strong>"));
  assert.ok(choice.includes("listening since 1h ago"), page);
  assert.ok(!page.includes('class="tag"'), page);
});

test("a worker picked before it stopped listening stays the pick, as not listening now", () => {
  const offered = picks([], [], now);
  const choices = kept(offered, "worker:bun-pins");
  assert.deepEqual(choices.map((p) => [p.key, p.target, p.live]), [["worker:bun-pins", { worker: "bun-pins" }, false]]);
  assert.deepEqual(drafted(fields(), choices, "").body.worker, "bun-pins");
  assert.deepEqual(kept(offered, "label:ops:east").map((p) => p.target), [{ label: "ops:east" }]);
  // Nothing picked, another, or a pick still offered, leaves the choices as they are.
  const listed = picks([listener("bun-pins", [])], [], now);
  for (const key of ["", "other", "worker:bun-pins"]) assert.equal(kept(listed, key), listed, key);
});

test("Enter alone in a field of the composer does not send it; Cmd or Ctrl+Enter sends it once", () => {
  const form = { sent: 0, requestSubmit() { this.sent++; } };
  const press = (extra = {}) => {
    let prevented = false;
    onKey({ key: "Enter", metaKey: false, ctrlKey: false, repeat: false, isComposing: false, target: { tagName: "INPUT" }, currentTarget: form, preventDefault() { prevented = true; }, ...extra });
    return prevented;
  };
  // Enter in the filter, the name or on a radio would send the form.
  assert.equal(press(), true);
  assert.equal(form.sent, 0);
  // In the message it starts a new line; on the button it presses it.
  assert.equal(press({ target: { tagName: "TEXTAREA" } }), false);
  assert.equal(press({ target: { tagName: "BUTTON" } }), false);
  assert.equal(press({ metaKey: true }), true);
  assert.equal(press({ ctrlKey: true, target: { tagName: "TEXTAREA" } }), true);
  assert.equal(form.sent, 2);
  // A held key sends once, and a key that composes text is left alone.
  press({ metaKey: true, repeat: true });
  assert.equal(press({ metaKey: true, isComposing: true }), false);
  assert.equal(form.sent, 2);
});

test("the choices not listening fold away while some are listening, and are the whole list while none is", () => {
  const cases = [kase("a1", "docs", ["docs"], 30), kase("a2", "ghost", [], 30)];
  // With nobody listening they are the whole list, and are not folded.
  const alone = draw({ cases });
  assert.ok(!alone.includes('class="fold"'), alone);
  assert.ok(alone.includes("not listening now: it waits until one is") && alone.includes("<strong>ghost</strong>") && alone.includes('name="filter"'), alone);
  const folded = draw({ listeners: [listener("lead", [])], cases });
  assert.ok(folded.includes('aria-expanded="false">3 more, not listening</button>') && !folded.includes("<strong>ghost</strong>"), folded);
});

test("the filter leaves the choices whose name has it, whatever its case, and the one picked", () => {
  const choices = picks([], [kase("a1", "docs", ["Deploy"], 30), kase("a2", "ghost", ["ops"], 30)], now);
  const keys = (text, key = "") => filtered(choices, text, key).map((p) => p.key);
  assert.deepEqual(keys(""), ["worker:docs", "worker:ghost", "label:Deploy", "label:ops"]);
  assert.deepEqual(keys("  "), keys(""));
  assert.deepEqual(keys("dep"), ["label:Deploy"]);
  assert.deepEqual(keys(" OS "), ["worker:ghost"]);
  assert.deepEqual(keys("zzz"), []);
  assert.deepEqual(keys("dep", "worker:ghost"), ["worker:ghost", "label:Deploy"]);
});

test("while who listens is not known, the composer lists the recent workers and labels plainly, saying nothing of who listens", () => {
  const page = draw({ listeners: null, cases: [kase("a1", "docs", ["docs"], 30)] });
  assert.ok(page.includes("<strong>docs</strong>") && page.includes('<span class="tag">docs</span>'), page);
  for (const claim of ["listening now", "not listening", 'class="fold"']) assert.ok(!page.includes(claim), `${claim}: ${page}`);
});

// A history of addresses, as the browser keeps one, for Entry.
function places(start) {
  const stack = [{ state: null, url: start }];
  let at = 0;
  const calls = [];
  return {
    calls,
    get url() { return stack[at].url; },
    get length() { return stack.length; },
    pushState(state, _, url) { calls.push("push"); stack.splice(at + 1); stack.push({ state, url: url ?? stack[at].url }); at++; },
    replaceState(state, _, url) { calls.push("replace"); stack[at] = { state, url: url ?? stack[at].url }; },
    back() { calls.push("back"); if (at > 0) at--; },
  };
}

test("an open composer has an entry at its page's address, which back takes off and a close leaves as it was", () => {
  const h = places("/");
  const entry = new Entry(h);
  entry.opened();
  entry.opened();
  assert.deepEqual([h.calls, h.length, h.url], [["push"], 2, "/"]);
  // Closed by Escape, send or the backdrop: the entry comes off with back.
  entry.closed();
  entry.closed();
  assert.deepEqual([h.calls, h.url], [["push", "back"], "/"]);
  // The back a close makes is not the composer's to answer.
  assert.equal(entry.popped(), false);
  // Back took it off: the composer's to answer, and a close adds no back.
  entry.opened();
  h.back();
  assert.equal(entry.popped(), true);
  entry.closed();
  assert.deepEqual(h.calls, ["push", "back", "push", "back"]);
  assert.equal(entry.popped(), false);
});

test("a composer opened again before the back its close made lands keeps its entry", () => {
  const h = places("/");
  const entry = new Entry(h);
  entry.opened({ to: "a", re: "" });
  entry.closed();
  // Opened again at once: the entry waits for the back to land.
  entry.opened({ to: "b", re: "" });
  assert.deepEqual(h.calls, ["push", "back"]);
  assert.equal(entry.popped(), false);
  assert.deepEqual([h.calls, h.length, entry.on], [["push", "back", "push"], 2, true]);
  // Closed before the back landed, it does not come back.
  const g = places("/");
  const other = new Entry(g);
  other.opened();
  other.closed();
  other.opened();
  other.closed();
  assert.equal(other.popped(), false);
  assert.deepEqual([g.calls, other.on], [["push", "back"], false]);
});

test("forward to the entry of a composer closed before makes it the composer's again", () => {
  const h = places("/");
  const entry = new Entry(h);
  entry.opened({ to: "homeday", re: "" });
  h.back();
  assert.equal(entry.popped(), true);
  entry.adopted({ to: "homeday", re: "" });
  assert.equal(entry.on, true);
  // Its close takes it off, as any close does.
  entry.closed();
  assert.deepEqual(h.calls, ["push", "back", "back"]);
});

test("back that asks about a note puts the entry back, and a link in the composer takes its place", () => {
  const h = places("/cases/UkLWZ");
  const entry = new Entry(h);
  entry.opened();
  h.back();
  assert.equal(entry.popped(), true);
  // Asked, the entry goes back on, so a second back asks again.
  entry.kept();
  assert.deepEqual([h.length, h.url], [2, "/cases/UkLWZ"]);
  h.back();
  assert.equal(entry.popped(), true);
  entry.kept();
  // "about" goes to the case in the entry's place: one move, no back after.
  assert.equal(entry.moved("/cases/p6klV-today-s-plan"), true);
  assert.deepEqual([h.url, h.length], ["/cases/p6klV-today-s-plan", 2]);
  entry.closed();
  assert.equal(h.calls.at(-1), "replace");
  // With no entry on, a move is the app's own.
  assert.equal(entry.moved("/"), false);
});

test("back does what Escape does: closes the composer, or asks when a note is written", () => {
  const back = { current: null };
  render(html`<${Composer} to="bun-pins" re="" listeners=${[]} cases=${[]} now=${now} onSent=${() => {}} onClose=${() => {}} back=${back} />`);
  // With no note written, it closes rather than asks.
  assert.equal(back.current(), false);
});
