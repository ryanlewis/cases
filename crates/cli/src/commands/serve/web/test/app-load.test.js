// What the inbox's App (static/app.ts) does as the page loads and moves:
// how start() mounts it, what the first read asks serve for and draws, each
// way that read can fail, each address and what a move between them does,
// back and forward included. The pins of characterisation tests: they say
// what App does today, to hold it still while it is split up. App runs on
// the stand-in page of app-page.js, which must be the first to load it.
import { test } from "node:test";
import assert from "node:assert/strict";
import { LISTENERS, OTHERS, WAITING, card, first, link, openApp } from "./app-page.js";
import { decision, steps } from "./case.js";

// A case `id` titled `title`, open and due today unless `fields` say
// otherwise.
const kase = (id, title, fields = {}) => ({ ...decision("open", steps.open), id, title, ...fields });
const a = kase("aB3xQ", "First case");
const b = kase("bC4yR", "Second case");
const shut = kase("cD5zS", "Closed case", { state: "closed", updated_at: "2026-09-25T13:30:00Z" });

// The ids of the cases the page's data holds, by part, and the one shown.
function shows(p) {
  const d = p.page.props.data;
  return { route: d.route, show: d.show, inbox: d.inbox.map((c) => c.id), done: d.done.map((c) => c.id), shown: d.shown && (d.shown.case ? d.shown.case.id : `missing ${d.shown.id}`) };
}

test("start() mounts App into the body, with the build the page says, and leaves the scroll to it", async () => {
  assert.equal(first.build, "b1");
  assert.equal(first.scrollRestoration, "manual");
  assert.equal(first.parent, globalThis.document.body);
  assert.deepEqual(first.requests, []);
});

test("the first load of / reads the two lists and the listeners, once each, and shows the first case", async () => {
  const p = await openApp({ server: { waiting: [a, b], done: [shut], listeners: [] } });
  assert.deepEqual(p.requests, [WAITING, OTHERS, LISTENERS]);
  assert.deepEqual(shows(p), { route: { page: "list", show: "inbox" }, show: "inbox", inbox: ["aB3xQ", "bC4yR"], done: ["cD5zS"], shown: "aB3xQ" });
  assert.deepEqual(p.page.props.data.listeners, []);
  assert.ok(p.html.includes('<a href="/" class="on" aria-current="page">inbox</a>'), p.html);
  assert.ok(p.html.includes('<header class="case-head">'), p.html);
  assert.ok(!p.html.includes('role="alert">'), p.html);
  // The title and the tab icon count the open cases.
  assert.equal(p.document.title, "(2) cases");
  assert.equal(p.favicon.getAttribute("href"), "/favicon.svg?n=2");
  // A first page starts at its top; the address is left as it was.
  assert.deepEqual(p.log, ["scrollTo 0"]);
  assert.equal(p.page.props.now, Date.parse("2026-09-25T13:45:00Z"));
  p.app.unmount();
});

test("an empty inbox draws the inbox-zero panel and a title with no count", async () => {
  const p = await openApp({ server: { done: [shut] } });
  assert.equal(shows(p).shown, null);
  assert.ok(p.html.includes('<div class="zero-panel">'), p.html);
  assert.equal(p.document.title, "cases");
  assert.equal(p.favicon.getAttribute("href"), "/favicon.svg");
  p.app.unmount();
});

test("nothing is drawn under the header before the first read comes back", async () => {
  let release;
  const p = await openApp({ server: { waiting: [a], routes: { [WAITING]: () => new Promise((r) => (release = r)) } } });
  assert.equal(p.page, undefined);
  assert.ok(p.html.includes('<main><div class="new-build-live" aria-live="polite"></div></main>'), p.html);
  assert.equal(p.document.title, "");
  release({ status: 200, body: { list: [a] } });
  await p.settle();
  assert.equal(shows(p).shown, "aB3xQ");
  p.app.unmount();
});

test("a case's page reads the case too, and lists it under its own filter", async () => {
  const p = await openApp({ url: "/cases/cD5zS-closed-case", server: { waiting: [a], done: [shut] } });
  assert.deepEqual(p.requests, [WAITING, OTHERS, "GET /v1/cases/cD5zS", LISTENERS]);
  assert.deepEqual(shows(p), { route: { page: "case", id: "cD5zS", show: null }, show: "done", inbox: ["aB3xQ"], done: ["cD5zS"], shown: "cD5zS" });
  // Its filter given, the case is listed under it.
  const q = await openApp({ url: "/cases/cD5zS?show=all", server: { waiting: [a], done: [shut] } });
  assert.equal(shows(q).show, "all");
  p.app.unmount();
  q.app.unmount();
});

test("a case serve has none of is shown as missing, under the inbox", async () => {
  const p = await openApp({ url: "/cases/zZ9yY", server: { waiting: [a] } });
  assert.deepEqual(shows(p), { route: { page: "case", id: "zZ9yY", show: null }, show: "inbox", inbox: ["aB3xQ"], done: [], shown: "missing zZ9yY" });
  assert.ok(p.html.includes('<p class="empty">no case zZ9yY in this store.</p>'), p.html);
  p.app.unmount();
});

test("a read serve refuses says serve's reason, and a read that never reached it says so", async () => {
  const store = () => ({ status: 500, body: { error: { category: "store", message: "the store cannot be read" } } });
  for (const route of [WAITING, OTHERS, "GET /v1/cases/aB3xQ"]) {
    const p = await openApp({ url: "/cases/aB3xQ", server: { waiting: [a], routes: { [route]: store } } });
    assert.ok(p.html.includes('<p class="error notice" role="alert">the store cannot be read</p>'), `${route}: ${p.html}`);
    assert.equal(p.page, undefined, route);
    p.app.unmount();
  }
  // A refusal with no body serve wrote says its status.
  const bare = await openApp({ server: { routes: { [WAITING]: () => ({ status: 502 }) } } });
  assert.ok(bare.html.includes('role="alert">serve answered 502</p>'), bare.html);
  bare.app.unmount();
  const down = await openApp({ server: { down: true } });
  assert.ok(down.html.includes('<p class="error notice" role="alert">serve cannot be reached. the inbox follows the store again once it is back.</p>'), down.html);
  assert.equal(down.page, undefined);
  down.app.unmount();
});

test("the listeners failing does not fail the page: who listens is not known", async () => {
  const p = await openApp({ server: { waiting: [a], routes: { [LISTENERS]: () => ({ status: 500, body: null }) } } });
  assert.equal(shows(p).shown, "aB3xQ");
  assert.equal(p.page.props.data.listeners, null);
  assert.ok(!p.html.includes('role="alert">'), p.html);
  p.app.unmount();
});

test("a failed read keeps what was shown, says why, and the next read that works clears it", async () => {
  const p = await openApp({ server: { waiting: [a] } });
  p.server.down = true;
  p.stream.onmessage({ data: "v2" });
  await p.settle();
  assert.ok(p.html.includes("serve cannot be reached."), p.html);
  assert.equal(shows(p).shown, "aB3xQ");
  p.server.down = false;
  p.stream.onmessage({ data: "v3" });
  await p.settle();
  assert.ok(!p.html.includes('role="alert">'), p.html);
  p.app.unmount();
});

test("the addresses: /done is put in the history as its filter's, an unknown one is the inbox", async () => {
  const done = await openApp({ url: "/done", server: { done: [shut] } });
  assert.deepEqual(done.log.slice(0, 1), ["replace /?show=done"]);
  assert.deepEqual(shows(done).route, { page: "list", show: "done" });
  assert.equal(shows(done).shown, "cD5zS");
  assert.equal(done.location.pathname + done.location.search, "/?show=done");
  done.app.unmount();
  const flight = await openApp({ url: "/done?show=inflight#top" });
  assert.deepEqual(flight.log.slice(0, 1), ["replace /?show=inflight#top"]);
  assert.deepEqual(shows(flight).route, { page: "list", show: "inflight" });
  flight.app.unmount();
  const lost = await openApp({ url: "/nowhere" });
  assert.deepEqual(shows(lost).route, { page: "list", show: "inbox" });
  assert.equal(lost.location.pathname, "/nowhere");
  lost.app.unmount();
  const filtered = await openApp({ url: "/?show=all", server: { waiting: [a], done: [shut] } });
  assert.equal(shows(filtered).show, "all");
  filtered.app.unmount();
});

test("the composer's address opens it over the list, and the address bar says the list's", async () => {
  const p = await openApp({ url: "/message?show=done&to=w1&re=aB3xQ-first-case", server: { done: [shut] } });
  assert.deepEqual(shows(p).route, { page: "list", show: "done" });
  assert.deepEqual(p.log.slice(0, 2), ["replace /?show=done", "replace /?show=done"]);
  // Its entry goes on the history over the list's, holding who and what.
  assert.deepEqual(p.entries(), ["/?show=done", ">/?show=done"]);
  assert.deepEqual(p.history.state, { composer: { to: "w1", re: "aB3xQ" } });
  assert.equal(p.composer.props.to, "w1");
  assert.equal(p.composer.props.re, "aB3xQ");
  assert.ok(p.html.includes('<div id="compose">'), p.html);
  // With no filter, over the inbox.
  const q = await openApp({ url: "/message" });
  assert.deepEqual(shows(q).route, { page: "list", show: "inbox" });
  assert.equal(q.composer.props.to, "");
  p.app.unmount();
  q.app.unmount();
});

test("a link to another page moves to it in the page: pushed, read, drawn from its top", async () => {
  const p = await openApp({ server: { waiting: [a, b] } });
  p.taken();
  p.log.length = 0;
  p.window.scrollY = 120;
  p.list.scrollTop = 30;
  p.work.scrollTop = 40;
  assert.equal(await p.click(link("/cases/bC4yR-second-case?show=inbox")), true);
  assert.deepEqual(p.log, ["replace /", "push /cases/bC4yR-second-case?show=inbox", "scrollTo 0"]);
  assert.deepEqual(p.entries(), ["/", ">/cases/bC4yR-second-case?show=inbox"]);
  assert.deepEqual(p.taken(), [WAITING, OTHERS, "GET /v1/cases/bC4yR", LISTENERS]);
  assert.deepEqual(shows(p).route, { page: "case", id: "bC4yR", show: "inbox" });
  assert.equal(shows(p).shown, "bC4yR");
  assert.equal(p.work.scrollTop, 0);
  // The list keeps its own scroll on a move.
  assert.equal(p.list.scrollTop, 30);
  p.app.unmount();
});

test("the links App leaves to the browser", async () => {
  const p = await openApp({ url: "/cases/aB3xQ", server: { waiting: [a, b] } });
  p.taken();
  const left = [
    [link("/cases/bC4yR"), { button: 1 }],
    [link("/cases/bC4yR"), { metaKey: true }],
    [link("/cases/bC4yR"), { ctrlKey: true }],
    [link("/cases/bC4yR"), { shiftKey: true }],
    [link("/cases/bC4yR"), { altKey: true }],
    [link("/cases/bC4yR"), { defaultPrevented: true }],
    [link("/cases/bC4yR", { target: "_blank" }), {}],
    [link("/cases/bC4yR", { download: true }), {}],
    [link("https://example.com/cases/bC4yR"), {}],
    [link("/favicon.svg"), {}],
    [link("/cases/aB3xQ#thread"), {}],
    [{ closest: undefined }, {}],
    [{ closest: () => null }, {}],
  ];
  for (const [target, mods] of left) {
    const before = p.entries().join(" ");
    await p.click(target, mods);
    assert.equal(p.entries().join(" "), before, JSON.stringify(mods));
  }
  assert.deepEqual(p.taken(), []);
  // The same page without a fragment is a move, which reads it again.
  assert.equal(await p.click(link("/cases/aB3xQ")), true);
  assert.deepEqual(p.taken(), [WAITING, OTHERS, "GET /v1/cases/aB3xQ", LISTENERS]);
  p.app.unmount();
});

test("back and forward move the page, and back returns to where it was scrolled", async () => {
  const p = await openApp({ server: { waiting: [a, b] } });
  p.window.scrollY = 120;
  p.list.scrollTop = 30;
  p.work.scrollTop = 40;
  await p.click(link("/cases/bC4yR"));
  p.taken();
  p.log.length = 0;
  await p.back();
  assert.deepEqual(shows(p).route, { page: "list", show: "inbox" });
  assert.deepEqual(p.taken(), [WAITING, OTHERS, LISTENERS]);
  assert.deepEqual(p.log, ["scrollTo 120"]);
  assert.equal(p.list.scrollTop, 30);
  assert.equal(p.work.scrollTop, 40);
  // Forward to a page that kept no scroll starts at its top.
  p.log.length = 0;
  await p.forward();
  assert.deepEqual(shows(p).route, { page: "case", id: "bC4yR", show: null });
  assert.deepEqual(p.taken(), [WAITING, OTHERS, "GET /v1/cases/bC4yR", LISTENERS]);
  assert.deepEqual(p.log, ["scrollTo 0"]);
  assert.equal(p.work.scrollTop, 0);
  assert.equal(p.list.scrollTop, 30);
  p.app.unmount();
});

test("a move in the history within the page shown, to a fragment and back, reads nothing", async () => {
  const p = await openApp({ server: { waiting: [a] } });
  p.history.pushState(null, "", "/#top");
  p.taken();
  await p.back();
  await p.forward();
  assert.deepEqual(p.taken(), []);
  p.app.unmount();
});

test("a read a later one overtakes is dropped, however late it comes back", async () => {
  const releases = [];
  const p = await openApp({ server: { waiting: [a, b] } });
  p.server.routes[WAITING] = () => new Promise((r) => releases.push(r));
  await p.click(link("/cases/aB3xQ"));
  await p.click(link("/cases/bC4yR"));
  assert.equal(releases.length, 2);
  releases[1]({ status: 200, body: { list: [a, b] } });
  await p.settle();
  assert.equal(shows(p).shown, "bC4yR");
  releases[0]({ status: 200, body: { list: [a, b] } });
  await p.settle();
  assert.equal(shows(p).shown, "bC4yR");
  // Nor does an overtaken read that failed say so.
  p.server.routes[WAITING] = () => new Promise((r) => releases.push(r));
  await p.click(link("/cases/aB3xQ"));
  await p.click(link("/cases/bC4yR"));
  releases[3]({ status: 200, body: { list: [a, b] } });
  await p.settle();
  releases[2]({ status: 500, body: { error: { category: "store", message: "late" } } });
  await p.settle();
  assert.ok(!p.html.includes("late"), p.html);
  p.app.unmount();
});

test("a refresh keeps the case shown on the list while it is listed; a move to the list picks the first again", async () => {
  const p = await openApp({ server: { waiting: [a, b] } });
  assert.equal(shows(p).shown, "aB3xQ");
  const urgent = kase("eF6aT", "Urgent case", { urgency: "blocking" });
  p.server.waiting = [urgent, a, b];
  p.stream.onmessage({ data: "v2" });
  await p.settle();
  assert.equal(shows(p).shown, "aB3xQ");
  assert.equal(p.document.title, "(3) cases");
  assert.equal(p.favicon.getAttribute("href"), "/favicon.svg?n=3&blocking=1");
  // Once it is no longer listed, the first is shown.
  p.server.waiting = [urgent, b];
  p.stream.onmessage({ data: "v3" });
  await p.settle();
  assert.equal(shows(p).shown, "eF6aT");
  p.server.waiting = [urgent, a, b];
  p.stream.onmessage({ data: "v4" });
  await p.settle();
  assert.equal(shows(p).shown, "eF6aT");
  // A move to the list, by its link, starts from the first case.
  await p.click(link("/cases/aB3xQ"));
  await p.click(link("/"));
  assert.equal(shows(p).shown, "eF6aT");
  p.app.unmount();
});

test("a move to another page cross-fades where the browser can and motion is not reduced; a refresh does not", async () => {
  const fades = [];
  let reduced = false;
  const extra = (page) => {
    page.document.startViewTransition = (update) => {
      fades.push(page.location.pathname);
      update();
    };
    globalThis.matchMedia = (q) => ({ matches: q === "(prefers-reduced-motion: reduce)" && reduced });
  };
  const p = await openApp({ server: { waiting: [a, b] }, extra });
  assert.deepEqual(fades, ["/"]);
  p.stream.onmessage({ data: "v2" });
  await p.settle();
  assert.deepEqual(fades, ["/"]);
  await p.click(link("/cases/bC4yR"));
  assert.deepEqual(fades, ["/", "/cases/bC4yR"]);
  assert.equal(shows(p).shown, "bC4yR");
  reduced = true;
  await p.click(link("/cases/aB3xQ"));
  assert.deepEqual(fades, ["/", "/cases/bC4yR"]);
  assert.equal(shows(p).shown, "aB3xQ");
  p.app.unmount();
  delete globalThis.matchMedia;
});

test("a read overtaken while its cross-fade waits to draw is dropped", async () => {
  const waiting = [];
  const extra = (page) => {
    page.document.startViewTransition = (update) => waiting.push(update);
    globalThis.matchMedia = () => ({ matches: false });
  };
  const p = await openApp({ server: { waiting: [a, b] }, extra });
  waiting.shift()();
  await p.settle();
  await p.click(link("/cases/bC4yR"));
  await p.click(link("/cases/aB3xQ"));
  assert.equal(waiting.length, 2);
  waiting[1]();
  await p.settle();
  assert.equal(shows(p).shown, "aB3xQ");
  waiting[0]();
  await p.settle();
  assert.equal(shows(p).shown, "aB3xQ");
  p.app.unmount();
  delete globalThis.matchMedia;
});

test("j and k open the next and previous card through App's links", async () => {
  const p = await openApp({ server: { waiting: [a, b] } });
  p.found["#inbox a.card"] = [card(p, "/cases/aB3xQ-first-case", true), card(p, "/cases/bC4yR-second-case")];
  assert.equal(await p.key("j"), true);
  assert.deepEqual(shows(p).route, { page: "case", id: "bC4yR", show: null });
  p.found["#inbox a.card"] = [card(p, "/cases/aB3xQ-first-case"), card(p, "/cases/bC4yR-second-case", true)];
  await p.key("k");
  assert.deepEqual(shows(p).route, { page: "case", id: "aB3xQ", show: null });
  p.app.unmount();
});
