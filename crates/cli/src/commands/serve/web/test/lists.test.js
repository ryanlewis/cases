import { test } from "node:test";
import assert from "node:assert/strict";
import { render } from "preact-render-to-string";
import { html } from "../static/html.ts";
import { HUSH, List, UNHEARD, UNPICKED, Waiting, addressee, afterRead, counts, heard, lineFollows, lineFrom, listed, listensForCase, parts, quiet, takes, tally, unpicked, zero, Zero } from "../static/lists.ts";
import { mark } from "../static/fresh.ts";
import { decision, steps } from "./case.js";

const now = Date.parse("2026-09-26T12:00:00Z");
const at = (minutes) => new Date(now - minutes * 60000).toISOString();
const open = (id, urgency, worker, minutes, kind = "decision") => ({
  id, kind, urgency, state: "open", title: `case ${id}`, worker, opened_at: at(minutes), events: [], revision: 1, last_seq: 1,
});
const inbox = (cases) => render(html`<${List} inbox=${cases} done=${[]} show="inbox" selected="" now=${now} />`).replace(/^<div class="filter-row">.*?<\/div><div class="waiting-live" aria-live="polite"><\/div>/, "");
const list = (inboxCases, done, show, selected = "") => render(html`<${List} inbox=${inboxCases} done=${done} show=${show} selected=${selected} now=${now} />`);

test("the needs-you banner counts the blocking cases and links to the first", () => {
  const page = inbox([
    open("aa1", "today", "a", 5),
    open("bb2", "blocking", "billing-migrate", 25),
    open("cc3", "blocking", undefined, 8, "stuck"),
    { ...open("dd4", "blocking", "d", 60), state: "parked" },
  ]);
  assert.ok(page.startsWith('<div id="inbox"><p class="needs-you">'), page);
  assert.ok(page.includes("<strong>2 cases</strong> are blocking an agent."), page);
  assert.ok(page.includes("<small>billing-migrate has waited 25m, an agent 8m.</small>"), page);
  assert.ok(page.includes('<a href="/cases/bb2-case-bb2">start</a></p>'), page);
});

test("one blocking case is one case, and none is no banner", () => {
  assert.ok(inbox([open("bb2", "blocking", "w", 0)]).includes("<strong>1 case</strong> is blocking an agent. <small>w has waited under a minute.</small>"));
  assert.ok(!inbox([open("aa1", "today", "a", 5)]).includes("needs-you"));
});

test("each card, whatever its state, has its kind's slot first in its meta", () => {
  const slot = (kind) => `<div class="meta"><span class="kind-icon kind-${kind}" aria-hidden="true"></span>`;
  assert.ok(inbox([open("aa1", "today", "a", 5, "fyi")]).includes(slot("fyi")));
  const closed = { ...open("aa1", "today", "a", 5, "signoff"), state: "closed", updated_at: at(1) };
  assert.ok(list([], [closed], "done").includes(slot("signoff")));
});

// A case the human is through with, `minutes` after its last event.
const through = (id, state, minutes, extra = {}) => ({ ...open(id, "today", "w", 600), state, updated_at: at(minutes), ...extra });
const answered = (id, minutes) => through(id, "answered", minutes, { answer: { answered_at: at(minutes) } });
const pickedUp = (id, minutes, by) => through(id, "pickedup", minutes, { pickup: { picked_up_at: at(minutes), by } });
const closed = (id, minutes, outcome) => through(id, "closed", minutes, { close: { outcome, closed_at: at(minutes) }, events: [{ event: "close", at: at(minutes), data: { outcome } }] });
const withdrawn = (id, minutes, reason) => through(id, "withdrawn", minutes, { events: [{ event: "withdraw", at: at(minutes), data: { reason } }] });

const waiting = [open("o1", "whenever", "a", 5), open("o2", "blocking", "b", 9), { ...open("p1", "today", "c", 20), state: "parked" }];
// Two days before is before today in any zone; a minute or two before 12:00
// UTC is today in any zone but one whose midnight falls in those minutes.
const others = [answered("a1", 30), pickedUp("k1", 10, "gift-agent"), closed("c1", 2880, "**shipped** in v2\n\nmore"), withdrawn("w1", 1, "not needed"), closed("c2", 2, "")];

test("each filter lists its states in its order, and counts them", () => {
  const ids = (show) => listed(waiting, others, show, now).map((c) => c.id);
  assert.deepEqual(ids("inbox"), ["o2", "o1", "p1"]);
  assert.deepEqual(ids("inflight"), ["k1", "a1"]);
  assert.deepEqual(ids("done"), ["w1", "c2", "c1"]);
  assert.deepEqual(ids("all"), ["o2", "o1", "p1", "k1", "a1", "w1", "c2", "c1"]);
  assert.deepEqual(counts(waiting, others), { inbox: 3, inflight: 2, done: 3, all: 8, briefings: 0 });
});

test("done puts today's apart from those before, and all follows the inbox's groups", () => {
  const heads = (show) => parts(waiting, others, show, now).map((p) => `${p.label} ${p.cases.length}`);
  assert.deepEqual(heads("done"), ["Done today 2", "Before today 1"]);
  assert.deepEqual(heads("all"), ["Blocking 1", "Today 0", "Whenever 1", "Parked 1", "In flight 2", "Done today 2", "Before today 1"].filter((h) => !h.endsWith(" 0")));
  assert.deepEqual(parts(waiting, [], "done", now), []);
});

test("the filter row shows each filter with its count, the one shown marked", () => {
  const page = list(waiting, others, "done");
  assert.ok(page.startsWith('<div class="filter-row"><nav class="chips filters" aria-label="filter"><a href="/" class="waiting blocking">inbox <span class="count">3</span></a><a href="/?show=inflight">in flight <span class="count">2</span></a><a href="/?show=done" class="on" aria-current="page">done <span class="count">3</span></a><a href="/?show=all">all <span class="count">8</span></a><a href="/?show=briefings" class="tab-briefings">briefings <span class="count">0</span></a></nav>'), page);
  assert.ok(list(waiting, others, "inbox", "o1").includes('<a href="/" class="on" aria-current="true">inbox <span class="count">3</span></a>'));
});

test("the keys that move through the list, and list the keys, sit beside the filter row, readable", () => {
  const page = list(waiting, others, "inbox");
  const row = page.slice(0, page.indexOf('<div class="waiting-live"'));
  assert.ok(row.endsWith('</nav><p class="list-keys"><span class="key-hint"><kbd>j</kbd><kbd>k</kbd> move</span> <span class="key-hint"><kbd>?</kbd> keys</span></p></div>'), row);
  assert.equal(row.includes("aria-hidden"), false);
});

test("a card keeps the filter in its link, and is marked when it is the case shown", () => {
  const page = list(waiting, others, "done", "c2");
  assert.ok(page.includes('<a class="card state-closed selected" href="/cases/c2-case-c2?show=done" aria-current="page">'), page);
  assert.ok(page.includes('<a class="card state-withdrawn" href="/cases/w1-case-w1?show=done">'), page);
  assert.ok(list(waiting, others, "inbox").includes('<a class="card urgency-blocking" href="/cases/o2-case-o2">'));
  assert.ok(list(waiting, others, "all").includes('href="/cases/o2-case-o2?show=all"'));
});

test("a card says who has an in-flight case and since when, a closed one's outcome, and why one was withdrawn", () => {
  const flying = list([], others, "inflight");
  assert.ok(flying.includes('<span class="badge state">picked up</span>'), flying);
  assert.ok(flying.includes('<p class="ask flight">with <strong>gift-agent</strong> · picked up 10m ago</p>'), flying);
  // Answered 30 minutes ago and not picked up, which it says in place of its age.
  assert.ok(flying.includes('<p class="ask flight">with <strong>w</strong> · <span class="unpicked">not picked up · 30m</span></p>'), flying);
  const done = list([], others, "done");
  assert.ok(done.includes('<p class="ask">shipped in v2</p>'), done);
  assert.ok(done.includes('<p class="ask">not needed</p>'), done);
  assert.ok(!done.includes("urgency-"), "a done card has no urgency rail");
});

test("an empty filter says so, and the needs-you banner is only on the inbox filter", () => {
  assert.ok(list([], [], "inflight").endsWith('<div id="inbox"><p class="empty">nothing is with an agent.</p></div>'));
  assert.ok(list([], [], "done").includes('<p class="empty">nothing is done yet.</p>'));
  assert.ok(list([], [], "all").includes('<p class="empty">there are no cases yet.</p>'));
  assert.ok(list([], others, "inbox").includes('<p class="empty">nothing is waiting on you.</p>'));
  const blocked = [open("b1", "blocking", "b", 3)];
  assert.ok(list(blocked, others, "inbox").includes("needs-you"));
  for (const show of ["inflight", "done", "all"]) assert.ok(!list(blocked, others, show).includes("needs-you"), show);
});

test("on another filter, the waiting notice says how many cases wait on you and how many block, with a way to the inbox", () => {
  const page = list(waiting, others, "inflight");
  assert.ok(page.includes(
    '<div class="waiting-live" aria-live="polite"><p class="waiting-notice blocking"><span><strong>2</strong> waiting on you · <strong class="hot">1 blocking</strong></span><a href="/">go to inbox</a></p></div><div id="inbox">',
  ), page);
  // Parked cases are set aside, not waiting.
  const calm = list([open("o1", "today", "a", 5), { ...open("p1", "today", "c", 20), state: "parked" }], others, "done");
  assert.ok(calm.includes('<p class="waiting-notice"><span><strong>1</strong> waiting on you</span><a href="/">go to inbox</a></p>'), calm);
  assert.ok(list(waiting, others, "all").includes('class="waiting-notice blocking"'));
});

test("the waiting notice's live region is there but empty on the inbox, and when nothing waits", () => {
  const empty = '<div class="waiting-live" aria-live="polite"></div>';
  assert.ok(list(waiting, others, "inbox").includes(empty));
  assert.ok(list([{ ...open("p1", "today", "c", 20), state: "parked" }], others, "inflight").includes(empty));
  assert.ok(render(html`<${Waiting} t=${tally(waiting)} show="done" float />`).startsWith('<div class="waiting-live float" aria-live="polite"><p class="waiting-notice blocking">'));
});

test("on another filter, the inbox filter stands out while cases wait, in the blocking colour when one blocks", () => {
  const chip = (inboxCases, show) => /<a href="\/"(?: class="([^"]*)")?/.exec(list(inboxCases, others, show))[1];
  assert.equal(chip(waiting, "inflight"), "waiting blocking");
  assert.equal(chip([open("o1", "today", "a", 5)], "done"), "waiting");
  assert.equal(chip(waiting, "inbox"), "on");
  assert.equal(chip([], "all"), undefined);
});

// A case the human answered (or parked, or resumed) `minutes` ago, now in
// `state`.
const byHuman = (id, minutes, state = "closed", event = "answer") =>
  through(id, state, minutes, { events: [{ event, author: "human", at: at(minutes), data: {} }] });
const panel = (cases) => render(html`<${Zero} z=${zero(cases, now)} now=${now} />`);

test("inbox zero says nothing needs you, and in one line who did what, the closed count linking to the done filter", () => {
  const page = panel([byHuman("h1", 5), byHuman("h2", 2880), closed("c2", 2, "")]);
  assert.ok(page.includes("<h1>nothing needs you.</h1>"), page);
  assert.ok(page.includes(
    '<p class="who-did">you answered <strong>1</strong> today (<strong>2</strong> this week) · agents <a href="/?show=done">closed <strong>1</strong></a></p>',
  ), page);
  assert.ok(!page.includes("inbox zero"), page);
  assert.ok(!page.includes("still with an agent"), page);
});

test("inbox zero leaves out each part of its line that is zero, and falls back to the week", () => {
  assert.ok(panel([byHuman("h1", 5, "answered", "park")]).includes('<p class="who-did">you answered <strong>1</strong> today</p>'));
  assert.ok(panel([byHuman("h2", 2880)]).includes('<p class="who-did">you answered <strong>1</strong> this week</p>'));
  assert.ok(panel([closed("c2", 2, "")]).includes('<p class="who-did">agents <a href="/?show=done">closed <strong>1</strong></a></p>'));
  // Beside the week's figure, the closed count, which is today's, says so.
  assert.ok(panel([byHuman("h2", 2880), closed("c2", 2, "")]).includes(
    '<p class="who-did">you answered <strong>1</strong> this week · agents <a href="/?show=done">closed <strong>1</strong></a> today</p>',
  ));
  const none = panel([]);
  assert.ok(!none.includes("who-did"), none);
  assert.ok(none.includes("<p>nothing has needed you yet.</p>"), none);
});

test("inbox zero says when the human last answered only once that is over an hour ago", () => {
  assert.equal(zero([byHuman("h1", 60)], now).lastAnswer, "");
  assert.ok(!panel([byHuman("h1", 59)]).includes("last answer"));
  assert.ok(panel([byHuman("h1", 61)]).includes('<p class="age">last answer 1h ago</p>'));
  assert.ok(panel([byHuman("h1", 300), byHuman("h2", 400)]).includes('<p class="age">last answer 5h ago</p>'));
  // A park is not an answer.
  assert.ok(!panel([byHuman("h1", 300, "parked", "park")]).includes("last answer"));
});

test("inbox zero says when the human last answered if that was before midnight, however recent", () => {
  // 00:15 local on a Monday: an answer at 23:50 on the Sunday is in neither
  // the day's count nor the week's.
  const monday = new Date(2026, 8, 28, 0, 15).getTime();
  const answer = (minutes) => ({ ...open("h1", "today", "w", 600), state: "closed", events: [{ event: "answer", author: "human", at: new Date(monday - minutes * 60000).toISOString(), data: {} }] });
  const z = zero([answer(25)], monday);
  assert.equal(z.week, 0);
  assert.equal(z.lastAnswer, "25m");
  const page = render(html`<${Zero} z=${z} now=${monday} />`);
  assert.ok(page.includes('<p class="age">last answer 25m ago</p>'), page);
  assert.ok(!page.includes("nothing has needed you yet"), page);
  // Ten seconds after midnight, an answer twenty seconds before it.
  const midnight = new Date(2026, 8, 28, 0, 0, 10).getTime();
  const late = { ...answer(0), events: [{ event: "answer", author: "human", at: new Date(midnight - 20000).toISOString(), data: {} }] };
  assert.ok(render(html`<${Zero} z=${zero([late], midnight)} now=${midnight} />`).includes('<p class="age">last answer just now</p>'));
});

test("inbox zero lists the cases in flight, latest first, with who has each and since when", () => {
  const page = panel([answered("a1", 5), pickedUp("k1", 10, "gift-agent")]);
  assert.ok(page.includes(
    '<section class="in-flight" aria-labelledby="in-flight-head"><h2 class="section" id="in-flight-head">still with agents</h2><ul class="flights">'
      + '<li><a href="/cases/a1-case-a1">case a1</a><span class="flight">with <strong>w</strong> · answered 5m ago</span><button type="button" class="row-withdraw" aria-label="withdraw case a1">withdraw</button></li>'
      + '<li><a href="/cases/k1-case-k1">case k1</a><span class="flight">with <strong>gift-agent</strong> · picked up 10m ago</span><button type="button" class="row-withdraw" aria-label="withdraw case k1">withdraw</button></li></ul></section>',
  ), page);
  assert.ok(!page.includes('class="more"'), page);
});

test("inbox zero lists an answer not picked up first, so the five it shows never leave it out", () => {
  const newer = [1, 2, 3, 4, 5].map((m) => pickedUp(`k${m}`, m, "gift-agent"));
  const z = zero([answered("a1", 30), ...newer, answered("a2", 12)], now);
  assert.deepEqual(z.flights.map((w) => w.c.id), ["a2", "a1", "k1", "k2", "k3", "k4", "k5"]);
  const page = render(html`<${Zero} z=${z} now=${now} />`);
  assert.ok(page.indexOf("case a1") < page.indexOf("case k1") && page.includes('<li class="more"><a href="/?show=inflight">2 more in flight</a></li>'), page);
});

test("a case with an agent is quiet once it has gone more than an hour without an event", () => {
  assert.equal(HUSH, 60 * 60 * 1000);
  assert.equal(quiet(answered("a1", 60), now), "");
  assert.equal(quiet(answered("a1", 61), now), "1h");
  assert.equal(quiet(pickedUp("k1", 185, "gift-agent"), now), "3h");
  assert.equal(quiet({ ...answered("a1", 5), updated_at: undefined }, now), "");
});

test("inbox zero dims a quiet row and says how long it has been quiet, in place of its age", () => {
  const page = panel([pickedUp("k1", 185, "gift-agent"), answered("a1", 30)]);
  assert.ok(page.includes('<li class="quiet"><a href="/cases/k1-case-k1">case k1</a><span class="flight">with <strong>gift-agent</strong> · picked up · <span class="hush">quiet 3h</span></span>'), page);
  assert.ok(page.includes('<li><a href="/cases/a1-case-a1">case a1</a><span class="flight">with <strong>w</strong> · <span class="unpicked">not picked up · 30m</span>'), page);
});

test("inbox zero lists five cases in flight and links to the filter for the rest", () => {
  const flying = Array.from({ length: 7 }, (_, i) => answered(`f${i}`, i + 1));
  const z = zero(flying, now);
  assert.equal(z.flights.length, 7);
  const page = render(html`<${Zero} z=${z} now=${now} />`);
  const titles = [...page.matchAll(/<li><a href="[^"]+">(case f\d)<\/a>/g)].map((m) => m[1]);
  assert.deepEqual(titles, ["case f0", "case f1", "case f2", "case f3", "case f4"]);
  assert.ok(page.includes('<li class="more"><a href="/?show=inflight">2 more in flight</a></li></ul>'), page);
});

test("inbox zero has no in-flight section when nothing is in flight", () => {
  const page = panel([...waiting, closed("c2", 2, "")]);
  assert.ok(!page.includes("flights"), page);
  assert.ok(!page.includes("still with agents"), page);
});

// A message the human sent `minutes` ago, to `worker` and `labels`, that no
// agent has picked up.
const sent = (id, minutes, worker, labels) => ({ ...open(id, "today", worker, minutes, "message"), state: "sent", labels, updated_at: at(minutes) });
const listener = (worker, labels, extra = {}) => ({ worker, labels, waits: [{ labels, kinds: [] }], since: at(120), heard: at(0.1), ...extra });

test("in flight lists the messages sent and not picked up first, under their own heading, and counts them", () => {
  const messages = [sent("m1", 30, "bun-pins", []), sent("m2", 5, "", ["release"])];
  const heads = parts(waiting, [...others, ...messages], "inflight", now).map((p) => `${p.label} ${p.cases.map((c) => c.id).join(",")}`);
  assert.deepEqual(heads, ["Sent m2,m1", "In flight k1,a1"]);
  assert.deepEqual(counts(waiting, [...others, ...messages]), { inbox: 3, inflight: 4, done: 3, all: 10, briefings: 0 });
  assert.deepEqual(tally(waiting).waiting, 2, "a message is never the human's to answer");
});

test("a message's card says who it went to, when, and when no one listening would take it", () => {
  const page = list([], [sent("m1", 30, "bun-pins", ["release"])], "inflight");
  assert.ok(page.includes('<span class="badge state">sent</span>'), page);
  assert.ok(page.includes("<span>to bun-pins</span>"), page);
  assert.ok(page.includes("sent to <strong>bun-pins or label release</strong> · 30m ago"), page);
  const listened = render(html`<${List} inbox=${[]} done=${[sent("m1", 30, "bun-pins", [])]} show="inflight" selected="" now=${now} listeners=${[]} />`);
  assert.ok(listened.includes('<span class="hush">nobody is listening</span>'), listened);
  const taken = render(html`<${List} inbox=${[]} done=${[sent("m1", 30, "", ["release"])]} show="inflight" selected="" now=${now} listeners=${[listener("lead", ["release"])]} />`);
  assert.ok(!taken.includes("nobody is listening"), taken);
});

test("who a message goes to, and whether a listener would take it", () => {
  assert.equal(addressee({ worker: "bun-pins", labels: ["a", "b"] }), "bun-pins or label a or label b");
  assert.equal(addressee({ labels: ["a"] }), "label a");
  assert.ok(takes([listener("w", [])], { worker: "w" }));
  assert.ok(takes([listener("x", ["a"])], { worker: "w", labels: ["a"] }));
  assert.ok(!takes([listener("x", ["b"])], { worker: "w", labels: ["a"] }));
});

test("a listener is dimmed once it has gone unheard for 20 s", () => {
  assert.equal(heard(listener("w", [], { heard: new Date(now - UNHEARD).toISOString() }), now), "");
  assert.equal(heard(listener("w", [], { heard: new Date(now - UNHEARD - 1000).toISOString() }), now), "not heard from for 21s");
  assert.equal(heard(listener("w", [], { heard: at(3) }), now), "not heard from for 3m");
});

const zeroWith = (cases, listeners) => render(html`<${Zero} z=${zero(cases, now)} now=${now} listeners=${listeners} />`);

test("inbox zero lists the messages not picked up first, then the cases with agents, then who is listening", () => {
  const page = zeroWith(
    [sent("m1", 30, "ghost", []), pickedUp("k1", 10, "bun-pins")],
    [listener("bun-pins", ["release"], { doing: "pinning bun", doing_at: at(4) })],
  );
  const where = (text) => {
    const i = page.indexOf(text);
    assert.notEqual(i, -1, `inbox zero does not say ${JSON.stringify(text)}`);
    return i;
  };
  assert.ok(where("sent, not picked up") < where("still with agents"));
  assert.ok(where("still with agents") < where("listening now"));
  assert.ok(where("to <strong>ghost</strong> · sent 30m ago · ") < where('<span class="hush">nobody is listening for ghost</span>'));
  // The row of a case whose worker is listening says what it is doing.
  assert.ok(where("with <strong>bun-pins</strong>") < where("doing: pinning bun · 4m ago"));
  assert.ok(page.includes('<a class="row-message" href="/message?to=bun-pins" aria-label="message bun-pins">message</a>'), page);
  assert.ok(page.includes("listening for 2h"), page);
  assert.ok(page.includes('<span class="tag">release</span>'), page);
  assert.ok(page.includes('<p class="send-message"><a href="/message">send a message</a></p>'), page);
});

test("inbox zero lists five who listen and links to the composer for the rest, dimming one unheard", () => {
  const listeners = ["a", "b", "c", "d", "e", "f", "g"].map((w) => listener(w, [], w === "b" ? { heard: at(0.5) } : {}));
  const page = zeroWith([], listeners);
  assert.equal(page.match(/class="row-message"/g).length, 5);
  assert.ok(page.includes('<li class="more"><a href="/message">2 more listening</a></li>'), page);
  assert.ok(page.includes('<li class="quiet"><strong>b</strong>'), page);
  assert.ok(page.includes("not heard from for 30s"), page);
  assert.ok(!page.includes("nothing has needed you yet."), page);
});

test("inbox zero has no outbox or listening section when there are none", () => {
  const page = zeroWith([], []);
  assert.ok(!page.includes("sent, not picked up") && !page.includes("listening now"), page);
});

test("inbox zero lists five messages sent and not picked up, links to in flight for the rest, and never flags one for its age", () => {
  const messages = ["m1", "m2", "m3", "m4", "m5", "m6", "m7"].map((id, i) => sent(id, 2880 + i, "bun-pins", []));
  const page = zeroWith(messages, [listener("bun-pins", [])]);
  const outbox = page.slice(page.indexOf('id="outbox-head"'), page.indexOf('id="listening-head"'));
  assert.equal(outbox.match(/<li><a href="\/cases\/m/g).length, 5, outbox);
  assert.ok(outbox.includes('<li class="more"><a href="/?show=inflight">2 more sent</a></li>'), outbox);
  // Two days old, with someone listening who would take them: nothing marks them.
  assert.ok(!outbox.includes("quiet") && !outbox.includes("hush"), outbox);
});

test("the rows of inbox zero say what each worker is doing: a case's, found by its worker when it was picked up by another name, and a listener's", () => {
  const page = zeroWith([pickedUp("k1", 10, "sess-7")], [listener("w", ["release"], { doing: "pinning bun", doing_at: at(4) })]);
  const flights = page.slice(page.indexOf('id="in-flight-head"'), page.indexOf('id="listening-head"'));
  assert.ok(flights.includes("with <strong>sess-7</strong>"), flights);
  assert.ok(flights.includes('<span class="doing">doing: pinning bun · 4m ago</span>'), flights);
  const listening = page.slice(page.indexOf('id="listening-head"'));
  assert.ok(listening.includes('<span class="doing">doing: pinning bun · 4m ago</span>'), listening);
});

test("an answer is flagged as not picked up once it has waited ten minutes, and only while it is answered", () => {
  assert.equal(UNPICKED, 10 * 60 * 1000);
  assert.equal(unpicked(answered("a1", 9), now), "");
  assert.equal(unpicked(answered("a1", 10), now), "10m");
  assert.equal(unpicked({ ...answered("a1", 30), answer: undefined }, now), "");
  // Picked up, closed or withdrawn, however long ago, it has not waited.
  for (const c of [pickedUp("k1", 30, "gift-agent"), closed("c1", 30, "done"), withdrawn("w1", 30, "not needed")]) assert.equal(unpicked(c, now), "", c.state);
  const flying = render(html`<${List} inbox=${[]} done=${[answered("a9", 9), pickedUp("k1", 30, "gift-agent")]} show="inflight" selected="" now=${now} listeners=${[]} />`);
  assert.ok(!flying.includes("unpicked"), flying);
  assert.ok(flying.includes("with <strong>w</strong> · answered 9m ago</p>"), flying);
});

test("a case is listened for by its own worker, with no labels or one of its labels, not by a shared label alone", () => {
  const c = { ...answered("a1", 30), labels: ["release"] };
  assert.equal(listensForCase([listener("lead", ["release"])], c), false);
  assert.equal(listensForCase([listener("w", [])], c), true);
  assert.equal(listensForCase([listener("w", ["release", "web"])], c), true);
  assert.equal(listensForCase([listener("w", ["web"])], c), false);
  assert.equal(listensForCase([], c), false);
  // A message goes by its own rule, where a shared label is enough.
  assert.equal(takes([listener("lead", ["release"])], c), true);
});

test("each of a worker's waits is matched on its own, with its kinds", () => {
  const c = { ...answered("a1", 30), labels: ["release"] };
  const waits = (...list) => listener("w", [...new Set(list.flatMap((w) => w.labels))], { waits: list });
  // A wait on no label beside one on another label takes the worker's every case.
  assert.equal(listensForCase([waits({ labels: ["web"], kinds: [] }, { labels: [], kinds: [] })], c), true);
  assert.equal(listensForCase([waits({ labels: ["web"], kinds: [] })], c), false);
  // A wait takes only its kinds.
  assert.equal(listensForCase([waits({ labels: ["release"], kinds: ["message"] })], c), false);
  assert.equal(listensForCase([waits({ labels: ["release"], kinds: ["message", "decision"] })], c), true);
  // A message goes to any wait of its worker, or one on its label.
  assert.equal(takes([waits({ labels: [], kinds: ["message"] })], { worker: "w" }), true);
  assert.equal(takes([{ ...listener("x", ["release"]), waits: [{ labels: ["release"], kinds: [] }] }], { worker: "y", labels: ["release"] }), true);
});

test("a flagged answer says when no listener matches it, on its card and on inbox zero, and nothing more when one does", () => {
  const c = { ...answered("a1", 12), labels: ["release"] };
  const card = (listeners) => render(html`<${List} inbox=${[]} done=${[c]} show="inflight" selected="" now=${now} listeners=${listeners} />`);
  assert.ok(card([listener("lead", ["release"])]).includes('<p class="ask flight">with <strong>w</strong> · <span class="unpicked">not picked up · 12m</span> · <span class="hush" title="no listening session\'s wait matches this case\'s worker and labels">no listener matches</span></p>'));
  assert.ok(card([listener("w", [])]).includes('<p class="ask flight">with <strong>w</strong> · <span class="unpicked">not picked up · 12m</span></p>'));
  assert.ok(zeroWith([c], []).includes('<span class="flight">with <strong>w</strong> · <span class="unpicked">not picked up · 12m</span> · <span class="hush" title="no listening session\'s wait matches this case\'s worker and labels">no listener matches</span></span>'));
  const heard = zeroWith([c], [listener("w", [], { doing: "rebasing", doing_at: at(1) })]);
  assert.ok(heard.includes('<span class="flight">with <strong>w</strong> · <span class="unpicked">not picked up · 12m</span></span>'), heard);
  assert.ok(!heard.includes("no listener matches"), heard);
});

test("neither hint shows while who is listening is not known", () => {
  const c = { ...answered("a1", 12), labels: ["release"] };
  const m = sent("m1", 30, "bun-pins", []);
  for (const listeners of [null, undefined]) {
    const cards = render(html`<${List} inbox=${[]} done=${[c, m]} show="inflight" selected="" now=${now} listeners=${listeners} />`);
    assert.ok(cards.includes('<p class="ask flight">with <strong>w</strong> · <span class="unpicked">not picked up · 12m</span></p>'), cards);
    assert.ok(cards.includes("sent to <strong>bun-pins</strong> · 30m ago</p>"), cards);
    assert.ok(!cards.includes("no listener matches") && !cards.includes("nobody is listening"), cards);
    const panel = zeroWith([c, m], listeners);
    assert.ok(panel.includes('<span class="flight">with <strong>w</strong> · <span class="unpicked">not picked up · 12m</span></span>'), panel);
    assert.ok(panel.includes('<li><a href="/cases/m1-case-m1">case m1</a><span class="flight">to <strong>bun-pins</strong> · sent 30m ago</span></li>'), panel);
    assert.ok(!panel.includes("no listener matches") && !panel.includes("nobody is listening") && !panel.includes("listening now"), panel);
  }
  // Known to be no one, both say so.
  const none = zeroWith([c, m], []);
  assert.ok(none.includes("no listener matches") && none.includes("nobody is listening for bun-pins"), none);
});

test("a failed read of the listeners keeps those read last once, and a second in a row makes them not known", () => {
  const was = [listener("w", [])];
  const now1 = [listener("lead", ["release"])];
  // Not read yet: a failed read leaves who listens not known.
  assert.deepEqual(afterRead(null, null, 0), [null, 1]);
  // A read that works gives its list, even an empty one, and clears the count.
  assert.deepEqual(afterRead(now1, null, 3), [now1, 0]);
  assert.deepEqual(afterRead([], was, 1), [[], 0]);
  // One failed read keeps the list; the next makes it not known.
  assert.deepEqual(afterRead(null, was, 0), [was, 1]);
  assert.deepEqual(afterRead(null, was, 1), [null, 2]);
  assert.deepEqual(afterRead(null, null, 2), [null, 3]);
});

// A row's withdraw line, open on a case still with an agent (ST-14).
const answeredCase = decision("answered", steps.open, steps.answer);
const pickedUpCase = decision("pickedup", steps.open, steps.answer, steps.pickup);

test("a withdraw line refused as stale stays on the case it was shown until the human sends again", () => {
  // Drawn at answered; the send is refused, as the case was picked up, and
  // the refusal shows it picked up.
  const refused = lineFrom(pickedUpCase);
  // The stream then brings it, still picked up, with its question amended:
  // a revision the human has not seen.
  const amend = { seq: 4, author: "agent", event: "amend", from: "pickedup", file: "", at: steps.note.at, data: {} };
  const amended = { ...decision("pickedup", steps.open, steps.answer, steps.pickup, amend), amend_seq: 4 };
  const { line, said } = lineFollows(refused, amended);
  assert.deepEqual(line, { state: "pickedup", drawn: mark(pickedUpCase) });
  // The refusal is left as it was said.
  assert.equal(said, undefined);
  // The stream bringing the case the refusal showed changes nothing.
  assert.deepEqual(lineFollows(refused, pickedUpCase), { line: refused });
});

test("a withdraw line takes a new state only when the case just moved on to it", () => {
  const drawn = lineFrom(answeredCase);
  // Picked up since: the line takes the case as it is now, and says so.
  assert.deepEqual(lineFollows(drawn, pickedUpCase), {
    line: lineFrom(pickedUpCase),
    said: "this case was picked up since you pressed withdraw. withdraw it if you still mean to.",
  });
  // Picked up, reopened by the agent's follow-up, answered again and picked
  // up again: the line keeps its mark, so the withdraw is refused as stale.
  const reopened = { ...steps.note, data: { body: "Follow-up", followup: true } };
  const again = { ...steps.answer, seq: 5 };
  const pickupAgain = { ...steps.pickup, seq: 6 };
  const later = decision("pickedup", steps.open, steps.answer, steps.pickup, { ...reopened, from: "pickedup" }, again, pickupAgain);
  assert.deepEqual(lineFollows(drawn, later), {
    line: { state: "pickedup", drawn: mark(answeredCase) },
    said: "this case changed since you pressed withdraw, and is picked up now.",
  });
});

test("a withdraw line takes nothing from a case the stream brings late", () => {
  // The refusal showed the case picked up; the stream then brings it as it
  // was before, answered, and then picked up again as the refusal showed.
  const refused = lineFrom(pickedUpCase);
  assert.deepEqual(lineFollows(refused, answeredCase), { line: refused });
  assert.deepEqual(lineFollows(refused, pickedUpCase), { line: refused });
  // A refusal read before a pickup the stream already brought: the line
  // follows the case the stream brought, as it would have.
  assert.deepEqual(lineFollows(lineFrom(answeredCase), pickedUpCase).line, lineFrom(pickedUpCase));
});
