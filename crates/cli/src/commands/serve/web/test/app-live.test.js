// What the inbox's App (static/app.ts) does once its page is up: what a
// store event, the stream dropping and coming back, the minute tick and the
// 15 s poll ask serve for and redraw; a newer build of serve; the composer
// opening and closing over the page; a send recorded, refused or stale; the
// keys and the notifications that go through App. Characterisation tests:
// they pin what App does today, request counts included, to hold it still
// while it is split up and while the way it refetches is made cheaper. App
// runs on the stand-in page of app-page.js, which must be the first to load
// it.
import { mock, test } from "node:test";
import assert from "node:assert/strict";
import { INDEX, LISTENERS, NOW, OTHERS, WAITING, element, form, link, nodes, openApp } from "./app-page.js";
import { decision, steps } from "./case.js";

const kase = (id, title, fields = {}) => ({ ...decision("open", steps.open), id, title, ...fields });
const a = kase("aB3xQ", "First case");
const b = kase("bC4yR", "Second case");
const shut = kase("cD5zS", "Closed case", { state: "closed", updated_at: "2026-09-25T13:30:00Z" });
const listener = { worker: "w1", labels: [], waits: [{ labels: [], kinds: [] }], since: "2026-09-25T13:00:00Z", heard: "2026-09-25T13:44:50Z" };

const shown = (p) => {
  const s = p.page.props.data.shown;
  return s && (s.case ? s.case.id : `missing ${s.id}`);
};
const route = (p) => p.page.props.data.route;
// A store event, as the stream hands it on.
const event = async (p, version) => {
  p.stream.onmessage({ data: version });
  await p.settle();
};

test("the page holds one stream, from the start", async () => {
  const p = await openApp({ server: { waiting: [a] } });
  assert.deepEqual(p.streams.map((s) => s.url), ["/v1/changes"]);
  p.app.unmount();
  assert.equal(p.streams[0].closed, true);
});

test("a store event reads the two lists and the listeners again, once each, and redraws", async () => {
  const p = await openApp({ server: { waiting: [a] } });
  p.taken();
  p.log.length = 0;
  p.server.waiting = [a, b];
  await event(p, "v2");
  assert.deepEqual(p.taken(), [WAITING, OTHERS, LISTENERS]);
  assert.deepEqual(p.page.props.data.inbox.map((c) => c.id), ["aB3xQ", "bC4yR"]);
  assert.equal(p.document.title, "(2) cases");
  // A refresh leaves the scroll where it is, and the address.
  assert.deepEqual(p.log, []);
  // The same version again reads nothing.
  await event(p, "v2");
  assert.deepEqual(p.taken(), []);
  p.app.unmount();
});

test("a store event on a case's page reads the case too, and keeps the page", async () => {
  const p = await openApp({ url: "/cases/bC4yR", server: { waiting: [a, b] } });
  p.taken();
  p.server.waiting = [a, { ...b, revision: 9 }];
  await event(p, "v2");
  assert.deepEqual(p.taken(), [WAITING, OTHERS, "GET /v1/cases/bC4yR", LISTENERS]);
  assert.equal(p.page.props.data.shown.case.revision, 9);
  assert.deepEqual(route(p), { page: "case", id: "bC4yR", show: null });
  p.app.unmount();
});

test("the stream dropping and coming back checks serve's build and reads again; opening alone only checks", async () => {
  const p = await openApp({ server: { waiting: [a] } });
  p.taken();
  p.stream.onopen();
  await p.settle();
  assert.deepEqual(p.taken(), [INDEX]);
  p.stream.onerror();
  p.stream.onopen();
  await p.settle();
  assert.deepEqual(p.taken(), [INDEX, WAITING, OTHERS, LISTENERS]);
  // Once back, the next open is not a drop.
  p.stream.onopen();
  await p.settle();
  assert.deepEqual(p.taken(), [INDEX]);
  p.app.unmount();
});

test("a hidden tab closes its stream, and opens it again from the last version when shown", async () => {
  const p = await openApp({ server: { waiting: [a] } });
  await event(p, "v 2");
  p.document.hidden = true;
  p.document.dispatch("visibilitychange");
  assert.equal(p.streams[0].closed, true);
  // Hidden twice opens nothing.
  p.document.dispatch("visibilitychange");
  assert.equal(p.streams.length, 1);
  p.document.hidden = false;
  p.document.dispatch("visibilitychange");
  p.document.dispatch("visibilitychange");
  assert.deepEqual(p.streams.map((s) => s.url), ["/v1/changes", "/v1/changes?after=v%202"]);
  p.app.unmount();
  assert.equal(p.document.count("visibilitychange"), 0);
});

test("with no EventSource, no stream is opened, and the minute tick still follows the store", async () => {
  const p = await openApp({ server: { waiting: [a] }, extra: (page) => delete page.window.EventSource });
  assert.equal(p.streams.length, 0);
  p.taken();
  p.server.waiting = [a, b];
  await p.tick(60000);
  assert.deepEqual(p.page.props.data.inbox.map((c) => c.id), ["aB3xQ", "bC4yR"]);
  p.app.unmount();
});

test("the minute tick, while this tab holds a stream, moves the clock, checks the build and opens the stream again: it reads no case", async () => {
  const p = await openApp({ server: { waiting: [a] } });
  await event(p, "v1");
  p.taken();
  await p.tick(59999);
  assert.deepEqual(p.taken(), []);
  assert.equal(p.streams.length, 1);
  await p.tick(1);
  assert.deepEqual(p.taken(), [LISTENERS, INDEX]);
  assert.equal(p.page.props.now, Date.parse("2026-09-25T13:46:00Z"));
  // The stream is opened again from the version the page has.
  assert.deepEqual(p.streams.map((s) => [s.url, s.closed]), [["/v1/changes", true], ["/v1/changes?after=v1", false]]);
  // Its open checks nothing more: the tick did.
  p.stream.onopen();
  await p.settle();
  assert.deepEqual(p.taken(), []);
  await p.tick(120000);
  assert.deepEqual(p.taken(), [LISTENERS, INDEX, LISTENERS, INDEX]);
  assert.equal(p.streams.length, 4);
  p.app.unmount();
  await p.tick(60000);
  assert.deepEqual(p.taken(), []);
  assert.equal(p.streams.length, 4);
  assert.ok(p.streams.every((s) => s.closed));
});

test("a stream the tick opened again that drops before it opens reads again as any drop does; later opens check the build", async () => {
  const p = await openApp({ server: { waiting: [a] } });
  await event(p, "v1");
  await p.tick(60000);
  p.taken();
  p.stream.onerror();
  p.stream.onopen();
  await p.settle();
  assert.deepEqual(p.taken(), [INDEX, WAITING, OTHERS, LISTENERS]);
  // Hidden and shown, the stream opened then checks the build as before.
  for (const tick of [false, true]) {
    if (tick) {
      await p.tick(60000);
      p.stream.onopen();
      await p.settle();
    }
    p.taken();
    p.document.hidden = true;
    p.document.dispatch("visibilitychange");
    p.document.hidden = false;
    p.document.dispatch("visibilitychange");
    p.stream.onopen();
    await p.settle();
    assert.deepEqual(p.taken(), [INDEX], `after a tick: ${tick}`);
  }
  p.app.unmount();
});

test("a stream the browser gave up on is replaced on the tick, from the version the page has, and reads again when it opens", async () => {
  const p = await openApp({ server: { waiting: [a] } });
  await event(p, "v1");
  // A refused reconnect, such as a proxy's 502: the browser will not retry.
  const giveUp = () => {
    p.stream.readyState = EventSource.CLOSED;
    p.stream.onerror();
  };
  giveUp();
  p.taken();
  p.server.waiting = [a, b];
  // The tick still reads the store: the new stream may not come up.
  await p.tick(60000);
  assert.deepEqual(p.taken(), [WAITING, OTHERS, LISTENERS, INDEX]);
  assert.deepEqual(p.streams.map((s) => [s.url, s.closed]), [["/v1/changes", true], ["/v1/changes?after=v1", false]]);
  // Given up on again: replaced again, one stream each tick.
  giveUp();
  await p.tick(60000);
  assert.deepEqual(p.taken(), [WAITING, OTHERS, LISTENERS, INDEX]);
  assert.equal(p.streams.length, 3);
  assert.equal(p.streams.filter((s) => !s.closed).length, 1);
  // Back: a drop's open, which checks the build and reads what it missed.
  p.stream.onopen();
  await p.settle();
  assert.deepEqual(p.taken(), [INDEX, WAITING, OTHERS, LISTENERS]);
  // And pushes reach the page again.
  await event(p, "v2");
  assert.deepEqual(p.taken(), [WAITING, OTHERS, LISTENERS]);
  p.app.unmount();
});

test("a stream the browser gave up on is replaced on the tick while reads of the store are failing too", async () => {
  const p = await openApp({ server: { waiting: [a] } });
  await event(p, "v1");
  p.server.routes[OTHERS] = () => ({ status: 500, body: { error: { category: "store", message: "the store is busy" } } });
  await event(p, "v2");
  assert.ok(p.html.includes('role="alert">the store is busy</p>'), p.html);
  p.stream.readyState = EventSource.CLOSED;
  p.stream.onerror();
  await p.tick(60000);
  assert.deepEqual(p.streams.map((s) => [s.url, s.closed]), [["/v1/changes", true], ["/v1/changes?after=v2", false]]);
  p.app.unmount();
});

test("the tick, with the stream up, still reads who is listening, which moves no version", async () => {
  const p = await openApp({ url: "/cases/aB3xQ", server: { waiting: [a], listeners: [] } });
  await event(p, "v1");
  p.server.listeners = [listener];
  await p.tick(60000);
  assert.deepEqual(p.page.props.data.listeners.map((l) => l.worker), ["w1"]);
  // Two failed reads in a row and who listens is not known (afterRead).
  p.server.routes[LISTENERS] = () => ({ status: 500, body: null });
  await p.tick(60000);
  assert.deepEqual(p.page.props.data.listeners.map((l) => l.worker), ["w1"]);
  await p.tick(60000);
  assert.equal(p.page.props.data.listeners, null);
  p.app.unmount();
});

test("the tick reads the store again after a read that failed, though the stream is up", async () => {
  const p = await openApp({ server: { waiting: [a] } });
  await event(p, "v1");
  p.server.routes[OTHERS] = () => ({ status: 500, body: { error: { category: "store", message: "the store is busy" } } });
  p.server.waiting = [a, b];
  await event(p, "v2");
  assert.ok(p.html.includes('role="alert">the store is busy</p>'), p.html);
  delete p.server.routes[OTHERS];
  p.taken();
  await p.tick(60000);
  assert.deepEqual(p.taken(), [WAITING, OTHERS, LISTENERS, INDEX]);
  assert.ok(!p.html.includes('role="alert">'), p.html);
  assert.deepEqual(p.page.props.data.inbox.map((c) => c.id), ["aB3xQ", "bC4yR"]);
  // Read once it worked, the next tick opens the stream again.
  await p.tick(60000);
  assert.deepEqual(p.taken(), [LISTENERS, INDEX]);
  p.app.unmount();
});

test("a stream the tick opened again, closed before it opened, checks the build when it opens later", async () => {
  const p = await openApp({ server: { waiting: [a] } });
  await event(p, "v1");
  await p.tick(60000);
  p.document.hidden = true;
  p.document.dispatch("visibilitychange");
  p.taken();
  p.document.hidden = false;
  p.document.dispatch("visibilitychange");
  p.stream.onopen();
  await p.settle();
  assert.deepEqual(p.taken(), [INDEX]);
  p.app.unmount();
});

test("the minute tick on a case's page reads no case either", async () => {
  const p = await openApp({ url: "/cases/cD5zS", server: { done: [shut] } });
  await event(p, "v1");
  p.taken();
  await p.tick(60000);
  assert.deepEqual(p.taken(), [LISTENERS, INDEX]);
  p.app.unmount();
});

test("a change the stream lost is caught up when the tick opens it again", async () => {
  const p = await openApp({ server: { waiting: [a] } });
  await event(p, "v1");
  // The store moves, and the stream says nothing, as one that died without
  // a word would.
  p.server.waiting = [a, b];
  await p.tick(60000);
  p.taken();
  // serve answers the stream opened after v1 at once, as the store moved.
  p.stream.onopen();
  await event(p, "v2");
  assert.deepEqual(p.taken(), [WAITING, OTHERS, LISTENERS]);
  assert.deepEqual(p.page.props.data.inbox.map((c) => c.id), ["aB3xQ", "bC4yR"]);
  p.app.unmount();
});

test("the minute tick reads the store, as before, while this tab's stream is down or it holds none", async () => {
  // Dropped and not back: the store is read, and the build checked.
  const p = await openApp({ server: { waiting: [a] } });
  await event(p, "v1");
  p.stream.onerror();
  p.taken();
  p.server.waiting = [a, b];
  await p.tick(60000);
  assert.deepEqual(p.taken(), [WAITING, OTHERS, LISTENERS, INDEX]);
  assert.equal(p.streams.length, 1);
  assert.deepEqual(p.page.props.data.inbox.map((c) => c.id), ["aB3xQ", "bC4yR"]);
  // Once back, the tick opens it again instead.
  p.stream.onopen();
  await p.settle();
  p.taken();
  await p.tick(60000);
  assert.deepEqual(p.taken(), [LISTENERS, INDEX]);
  p.app.unmount();
  // A hidden tab, with no locks, holds no stream: it reads the store, and
  // reads it again as it is shown.
  const q = await openApp({ server: { waiting: [a] } });
  await event(q, "v1");
  q.document.hidden = true;
  q.document.dispatch("visibilitychange");
  q.taken();
  q.server.waiting = [a, b];
  await q.tick(60000);
  assert.deepEqual(q.taken(), [WAITING, OTHERS, LISTENERS, INDEX]);
  assert.deepEqual(q.page.props.data.inbox.map((c) => c.id), ["aB3xQ", "bC4yR"]);
  q.document.hidden = false;
  q.document.dispatch("visibilitychange");
  assert.equal(q.stream.url, "/v1/changes?after=v1");
  q.app.unmount();
});

test("an hour of ticks keeps ages and the zero panel's day right, and reads no case while the stream holds", async () => {
  // Ten minutes before the local midnight after NOW, with an answer given
  // five minutes before that.
  const midnight = new Date(NOW);
  midnight.setHours(24, 0, 0, 0);
  const now = midnight.getTime() - 10 * 60000;
  const at = new Date(now - 5 * 60000).toISOString();
  const answered = { ...decision("answered", { ...steps.open, at }, { ...steps.answer, at }), id: "gH8cW", title: "Answered late", answer: { answered_at: at } };
  const p = await openApp({ now, server: { done: [answered] } });
  await event(p, "v1");
  assert.ok(p.html.includes("you answered <strong>1</strong> today"), p.html);
  assert.ok(p.html.includes("answered 5m ago"), p.html);
  p.taken();
  for (let i = 0; i < 60; i++) await p.tick(60000);
  // The panel's 15 s poll still reads the listeners; nothing reads a case.
  const taken = p.taken();
  assert.equal(taken.filter((r) => r === INDEX).length, 60);
  assert.equal(taken.filter((r) => r === LISTENERS).length, 300);
  assert.equal(taken.length, 360);
  assert.equal(p.page.props.now, now + 3600000);
  // Past midnight the answer is this week's, not today's, and an hour older.
  assert.ok(!p.html.includes("today"), p.html);
  assert.ok(p.html.includes("you answered <strong>1</strong> this week"), p.html);
  assert.ok(p.html.includes('<p class="age">last answer 1h ago</p>'), p.html);
  assert.ok(p.html.includes('<span class="unpicked">not picked up · 1h</span>'), p.html);
  p.app.unmount();
});

test("the 15 s poll reads the listeners alone, while the inbox-zero panel or the composer shows them", async () => {
  const p = await openApp({ server: { listeners: [] } });
  p.taken();
  p.server.listeners = [listener];
  await p.tick(15000);
  assert.deepEqual(p.taken(), [LISTENERS]);
  assert.deepEqual(p.page.props.data.listeners.map((l) => l.worker), ["w1"]);
  assert.equal(p.page.props.now, Date.parse("2026-09-25T13:45:15Z"));
  // A failed read keeps who was heard for one more poll (afterRead).
  p.server.down = true;
  await p.tick(15000);
  assert.deepEqual(p.page.props.data.listeners.map((l) => l.worker), ["w1"]);
  await p.tick(15000);
  assert.equal(p.page.props.data.listeners, null);
  p.app.unmount();
  // On a case's page the poll reads nothing, until the composer opens.
  const q = await openApp({ url: "/cases/aB3xQ", server: { waiting: [a] } });
  q.taken();
  await q.tick(15000);
  assert.deepEqual(q.taken(), []);
  await q.click(link("/message"));
  await q.tick(15000);
  assert.deepEqual(q.taken(), [LISTENERS]);
  q.app.unmount();
});

test("a read of who is listening the 15 s poll has out does not overwrite a newer load's", async () => {
  const p = await openApp({ server: { listeners: [] } });
  let release;
  p.server.routes[LISTENERS] = () => new Promise((r) => (release = r));
  await p.tick(15000);
  delete p.server.routes[LISTENERS];
  p.server.listeners = [listener];
  await event(p, "v2");
  assert.deepEqual(p.page.props.data.listeners.map((l) => l.worker), ["w1"]);
  release({ status: 200, body: { list: [] } });
  await p.settle();
  assert.deepEqual(p.page.props.data.listeners.map((l) => l.worker), ["w1"]);
  p.app.unmount();
});

// A browser that shows notifications, the ones shown in `shownNotes`, for
// openApp's `extra`, and how many /v1/notifications requests `p` has made.
function notifying(t, shownNotes, stored) {
  t.after(() => delete globalThis.Notification);
  return (page) => {
    page.window.Notification = true;
    globalThis.Notification = class {
      static permission = "granted";
      constructor(title) {
        shownNotes.push(title);
      }
      close() {}
    };
    if (stored) page.stored.set(CURSOR, JSON.stringify(stored));
  };
}
const CURSOR = "cases.notify-cursor";
const reads = (p) => p.requests.filter((r) => r.includes("/v1/notifications")).length;

test("App's keys and its notifications stop when it goes", async (t) => {
  const p = await openApp({ prefs: { notify: "all" }, server: { waiting: [a] }, extra: notifying(t, []) });
  assert.equal(p.document.count("keydown"), 1);
  await p.tick(5000);
  assert.equal(reads(p), 2);
  p.app.unmount();
  assert.equal(p.document.count("keydown"), 0);
  await p.tick(600000);
  assert.equal(reads(p), 2);
});

test("a read of the notifications out when App goes shows nothing and leaves the cursor", async (t) => {
  const shownNotes = [];
  const was = { boot: "boot1", id: 0 };
  let release;
  const server = { waiting: [a], routes: { "GET /v1/notifications?after=0": () => new Promise((r) => (release = r)) } };
  const p = await openApp({ prefs: { notify: "all" }, server, extra: notifying(t, shownNotes, was) });
  p.app.unmount();
  release({ status: 200, body: { boot: "boot1", latest: 1, items: [{ id: 1, title: "Late", body: "", tag: "x", url: "/" }] } });
  await p.settle();
  assert.deepEqual(shownNotes, []);
  assert.deepEqual(JSON.parse(p.stored.get(CURSOR)), was);
});

test("a read of the list that lands while the back of a composer's close is out does not undo a refusal", async () => {
  const p = await openApp({ server: { waiting: [a, b] } });
  await p.click(link("/message?to=w1"));
  let release;
  p.server.routes[WAITING] = () => new Promise((r) => (release = r));
  await event(p, "v2");
  delete p.server.routes[WAITING];
  p.composer.props.onClose();
  p.page.props.refused({ ...a, revision: 7 });
  release({ status: 200, body: { list: [b] } });
  await p.settle();
  assert.deepEqual(route(p), { page: "case", id: "aB3xQ", show: "inbox" });
  assert.equal(shown(p), "aB3xQ");
  p.app.unmount();
});

test("a refusal on the list while the back of a composer's close is out stays on the refused case", async () => {
  const p = await openApp({ server: { waiting: [a, b] } });
  await p.click(link("/message?to=w1"));
  p.composer.props.onClose();
  p.page.props.refused({ ...a, revision: 7 });
  await p.settle();
  assert.deepEqual(route(p), { page: "case", id: "aB3xQ", show: "inbox" });
  // The composer's entry is left ahead, as any closed composer's is.
  assert.deepEqual(p.entries(), [">/cases/aB3xQ-first-case", "/"]);
  p.app.unmount();
});

// A page whose serve, once it is up, says it is build b2 from GET /v1,
// with `forms` as what typed() finds; `options` go to openApp, `server`
// over the two cases it lists.
async function upgraded(forms = [], options = {}) {
  const p = await openApp({ ...options, server: { waiting: [a, b], ...options.server } });
  p.server.build = "b2";
  p.found["#case form.respond, #compose form, form.row-confirm"] = forms;
  p.taken();
  return p;
}

test("serve a newer build with nothing typed: the page reloads, once, and checks no more", async () => {
  const p = await upgraded();
  p.stream.onopen();
  await p.settle();
  assert.deepEqual(p.left, ["reload"]);
  await p.tick(60000);
  p.stream.onopen();
  await p.settle();
  assert.deepEqual(p.left, ["reload"]);
  assert.equal(p.taken().filter((r) => r === INDEX).length, 1);
  p.app.unmount();
});

test("serve the same build: nothing changes", async () => {
  const p = await openApp({ server: { waiting: [a] } });
  p.stream.onopen();
  await p.settle();
  assert.deepEqual(p.left, []);
  assert.ok(!p.html.includes("new-build\""), p.html);
  p.app.unmount();
});

test("what counts as typed, which a reload would lose", async () => {
  const say = [
    form({ fields: ["a note"] }),
    form({ is: ['[data-write="amend"]'], edited: true }),
    form({ is: ["#case form"], checked: [{}] }),
    form({ is: ["#case form"], checked: [{ preset: true }, {}] }),
  ];
  const reload = [
    form(),
    form({ fields: ["  \n "] }),
    form({ is: ['[data-write="amend"]'], fields: ["the body"] }),
    form({ is: ["#case form"], checked: [{ preset: true }] }),
    form({ checked: [{}] }),
  ];
  for (const [forms, left] of [...say.map((f) => [[f], []]), ...reload.map((f) => [[f], ["reload"]]), [[reload[0], say[0]], []]]) {
    const p = await upgraded(forms);
    p.stream.onopen();
    await p.settle();
    assert.deepEqual(p.left, left, JSON.stringify(forms.map((f) => f.is)));
    assert.equal(p.html.includes("a new version of cases is running"), !left.length);
    p.app.unmount();
  }
});

test("serve a newer build while something is typed: the page says so, and the next move loads the page", async () => {
  const p = await upgraded([form({ fields: ["a note"] })]);
  p.stream.onopen();
  await p.settle();
  assert.ok(p.html.includes('<div class="new-build-live" aria-live="polite"><p class="new-build"><span>a new version of cases is running</span><button type="button">reload</button></p></div>'), p.html);
  assert.deepEqual(p.left, []);
  await p.click(link("/cases/bC4yR"));
  assert.deepEqual(p.left, ["assign /cases/bC4yR"]);
  assert.deepEqual(p.entries(), [">/"]);
  // The composer's address still opens the composer in the page.
  await p.click(link("/message"));
  assert.ok(p.composer);
  // With the composer's entry on, the page loaded takes its place.
  await p.click(link("/cases/bC4yR"));
  assert.deepEqual(p.left, ["assign /cases/bC4yR", "replace /cases/bC4yR"]);
  p.app.unmount();
});

test("the new build's reload button reloads", async () => {
  const p = await upgraded([form({ fields: ["a note"] })]);
  p.stream.onopen();
  await p.settle();
  const reload = [...nodes(p.tree)].find((v) => v.type === "button" && v.props.children === "reload");
  reload.props.onClick();
  assert.deepEqual(p.left, ["reload"]);
  p.app.unmount();
});

test("back to another page when serve is a newer build reloads it; back within the page does not", async () => {
  const p = await upgraded([form({ fields: ["a note"] })]);
  await p.click(link("/cases/bC4yR"));
  p.stream.onopen();
  await p.settle();
  p.history.pushState(null, "", "/cases/bC4yR#thread");
  await p.back();
  assert.deepEqual(p.left, []);
  await p.back();
  assert.deepEqual(p.left, ["reload"]);
  p.app.unmount();
});

test("a build check asked for while one is out runs once after it; one that fails changes nothing", async () => {
  const p = await openApp({ server: { waiting: [a] } });
  const releases = [];
  p.server.routes[INDEX] = () => new Promise((r) => releases.push(r));
  p.taken();
  p.stream.onopen();
  p.stream.onopen();
  p.stream.onopen();
  await p.settle();
  assert.deepEqual(p.taken(), [INDEX]);
  releases[0]({ status: 200, body: { version: "b1", csrf: "t" } });
  await p.settle();
  assert.deepEqual(p.taken(), [INDEX]);
  releases[1]({ status: 200, body: { version: "b1", csrf: "t" } });
  await p.settle();
  assert.deepEqual(p.taken(), []);
  // serve out of reach for a check: nothing is said, and the next asks.
  delete p.server.routes[INDEX];
  p.server.down = true;
  p.stream.onopen();
  await p.settle();
  p.server.down = false;
  p.server.build = "b2";
  p.stream.onopen();
  await p.settle();
  assert.deepEqual(p.left, ["reload"]);
  assert.ok(!p.html.includes('role="alert">'), p.html);
  p.app.unmount();
});

test("the composer opens over the page shown, with its own entry in the history, and closes off it", async () => {
  const p = await openApp({ url: "/cases/bC4yR", server: { waiting: [a, b], listeners: [listener] } });
  p.taken();
  p.window.scrollY = 50;
  assert.equal(await p.click(link("/message?to=w1&re=bC4yR")), true);
  assert.deepEqual(p.taken(), []);
  assert.deepEqual(route(p), { page: "case", id: "bC4yR", show: null });
  assert.deepEqual(p.entries(), ["/cases/bC4yR", ">/cases/bC4yR"]);
  assert.deepEqual(p.history.state, { composer: { to: "w1", re: "bC4yR" } });
  const c = p.composer;
  assert.deepEqual([c.props.to, c.props.re], ["w1", "bC4yR"]);
  assert.deepEqual(c.props.listeners.map((l) => l.worker), ["w1"]);
  assert.deepEqual(c.props.cases.map((x) => x.id), ["aB3xQ", "bC4yR"]);
  c.props.onClose();
  await p.settle();
  assert.equal(p.composer, undefined);
  assert.deepEqual(p.entries(), [">/cases/bC4yR", "/cases/bC4yR"]);
  // The page under it kept where it was scrolled.
  assert.deepEqual(p.history.state, { scrolled: { y: 50, list: 0, work: 0 } });
  await p.forward();
  assert.deepEqual(p.history.state, { composer: { to: "w1", re: "bC4yR" } });
  p.app.unmount();
});

test("the composer waits for the page's first read to be drawn", async () => {
  let release;
  const p = await openApp({ url: "/message", server: { routes: { [WAITING]: () => new Promise((r) => (release = r)) } } });
  assert.equal(p.composer, undefined);
  release({ status: 200, body: { list: [] } });
  await p.settle();
  assert.ok(p.composer);
  p.app.unmount();
});

test("the close of a composer another replaced does not close the new one", async () => {
  const p = await openApp({ server: { waiting: [a] } });
  await p.click(link("/message?to=w1"));
  const old = p.composer;
  await p.tick(1000);
  await p.click(link("/message?to=w2"));
  assert.equal(p.composer.props.to, "w2");
  assert.notEqual(p.composer.key, old.key);
  old.props.onClose();
  await p.settle();
  assert.equal(p.composer.props.to, "w2");
  // Its entry was put on once.
  assert.deepEqual(p.entries(), ["/", ">/"]);
  p.app.unmount();
});

test("back with the composer open is the composer's to answer", async () => {
  const p = await openApp({ server: { waiting: [a] } });
  await p.click(link("/message"));
  // It asks about the note written: its entry goes back on.
  let asked = 0;
  p.composer.props.back.current = () => (asked++, true);
  await p.back();
  assert.equal(asked, 1);
  assert.ok(p.composer);
  assert.deepEqual(p.entries(), ["/", ">/"]);
  // It closes by itself: App leaves it be.
  p.composer.props.back.current = () => (asked++, false);
  await p.back();
  assert.equal(asked, 2);
  assert.ok(p.composer);
  assert.deepEqual(p.entries(), [">/", "/"]);
  p.app.unmount();
  // One that set nothing for back goes.
  const q = await openApp({ server: { waiting: [a] } });
  await q.click(link("/message"));
  q.composer.props.back.current = null;
  await q.back();
  assert.equal(q.composer, undefined);
  q.app.unmount();
});

test("forward onto a closed composer's entry opens it again", async () => {
  const p = await openApp({ server: { waiting: [a] } });
  await p.click(link("/message?to=w1"));
  p.composer.props.back.current = null;
  await p.back();
  assert.equal(p.composer, undefined);
  await p.forward();
  assert.equal(p.composer.props.to, "w1");
  // It is the composer's entry again: a close takes it off.
  p.composer.props.onClose();
  await p.settle();
  assert.deepEqual(p.entries(), [">/", "/"]);
  p.app.unmount();
});

test("a composer opened while the back of the last one's close is out is not opened again when it lands", async () => {
  const p = await openApp({ server: { waiting: [a] } });
  await p.click(link("/message?to=w1"));
  p.composer.props.onClose();
  // Before the back lands, a link opens the composer again, and it is drawn.
  mock.timers.tick(1000);
  p.document.dispatch("click", { target: link("/message?to=w2"), button: 0, preventDefault() {} });
  p.app.draw();
  const key = p.composer.key;
  mock.timers.tick(1000);
  await p.settle();
  assert.equal(p.composer.props.to, "w2");
  assert.equal(p.composer.key, key);
  assert.deepEqual(p.entries(), ["/", ">/"]);
  assert.deepEqual(p.history.state, { composer: { to: "w2", re: "" } });
  p.app.unmount();
});

test("back onto the composer's address opens the composer", async () => {
  const p = await openApp({ url: "/message?to=w1", server: { waiting: [a] } });
  p.composer.props.onClose();
  await p.settle();
  p.history.pushState(null, "", "/message?to=w3");
  p.history.pushState(null, "", "/");
  await p.back();
  assert.equal(p.composer.props.to, "w3");
  p.app.unmount();
});

test("a link in the composer closes it; a move from outside it leaves it open over the new page", async () => {
  const p = await openApp({ server: { waiting: [a, b] } });
  await p.click(link("/message?re=bC4yR"));
  p.taken();
  await p.click(link("/cases/bC4yR", { composer: true }));
  assert.equal(p.composer, undefined);
  assert.deepEqual(route(p), { page: "case", id: "bC4yR", show: null });
  assert.deepEqual(p.entries(), ["/", ">/cases/bC4yR"]);
  assert.deepEqual(p.taken(), [WAITING, OTHERS, "GET /v1/cases/bC4yR", LISTENERS]);
  await p.click(link("/message?to=w1"));
  await p.click(link("/cases/aB3xQ"));
  assert.equal(p.composer.props.to, "w1");
  assert.deepEqual(route(p), { page: "case", id: "aB3xQ", show: null });
  // The page moved to took the composer's entry's place, and the entry
  // went on again over it.
  assert.deepEqual(p.entries(), ["/", "/cases/bC4yR", "/cases/aB3xQ", ">/cases/aB3xQ"]);
  assert.deepEqual(p.history.state, { composer: { to: "w1", re: "" } });
  p.app.unmount();
});

// The props of Recorded's paragraph as drawn now.
const recordedLine = (p) => p.recorded.type(p.recorded.props);

test("a send recorded says so and moves to the next case in the list, or to the list after the last", async () => {
  const p = await openApp({ server: { waiting: [a, b] } });
  p.page.props.sent("answer", a);
  await p.settle();
  assert.ok(p.html.includes('<p class="recorded" role="status"><span>answer recorded on <a href="/cases/aB3xQ-first-case">First case</a></span>'), p.html);
  assert.deepEqual(p.entries(), ["/", ">/cases/bC4yR-second-case"]);
  p.page.props.sent("park", b);
  await p.settle();
  assert.ok(p.html.includes("park recorded on"), p.html);
  assert.deepEqual(p.entries(), ["/", "/cases/bC4yR-second-case", ">/"]);
  p.app.unmount();
  // Under a filter, the move keeps it.
  const q = await openApp({ url: "/cases/cD5zS?show=done", server: { done: [shut, { ...shut, id: "dE6aU", title: "Older", updated_at: "2026-09-25T13:00:00Z" }] } });
  q.page.props.sent("withdraw", shut);
  await q.settle();
  assert.deepEqual(q.entries(), ["/cases/cD5zS?show=done", ">/cases/dE6aU-older?show=done"]);
  q.app.unmount();
});

test("a send of a case not in the list moves to the list", async () => {
  const p = await openApp({ url: "/cases/zZ9yY", server: { waiting: [a], cases: { zZ9yY: kase("zZ9yY", "Elsewhere", { state: "closed" }) } } });
  p.page.props.sent("answer", kase("zZ9yY", "Elsewhere"));
  await p.settle();
  assert.deepEqual(p.entries(), ["/cases/zZ9yY", ">/?show=done"]);
  p.app.unmount();
});

test("sends that stay where they are: an edit, a message, one from the inbox-zero panel, one after a move", async () => {
  const p = await openApp({ server: { waiting: [a, b] } });
  p.page.props.sent("amend", a);
  await p.settle();
  assert.ok(p.html.includes("<span>edit recorded on <a"), p.html);
  assert.deepEqual(p.entries(), [">/"]);
  await p.click(link("/message"));
  p.taken();
  p.composer.props.onSent(kase("fG7bV", "A note", { kind: "message", state: "sent", worker: "w1" }));
  await p.settle();
  assert.ok(p.html.includes('<span>message sent to w1: <a href="/cases/fG7bV-a-note">A note</a></span>'), p.html);
  assert.deepEqual(route(p), { page: "list", show: "inbox" });
  assert.deepEqual(p.entries(), ["/", ">/"]);
  assert.deepEqual(p.taken(), []);
  // A send that comes back after the page moved on only says so.
  const sent = p.page.props.sent;
  p.composer.props.onClose();
  await p.settle();
  await p.click(link("/?show=done"));
  sent("answer", a);
  await p.settle();
  assert.ok(p.html.includes("answer recorded on"), p.html);
  assert.deepEqual(route(p), { page: "list", show: "done" });
  p.app.unmount();
  const z = await openApp({ server: { done: [shut] } });
  z.page.props.sent("withdraw", shut);
  await z.settle();
  assert.deepEqual(z.entries(), [">/"]);
  z.app.unmount();
});

test("the line a send leaves goes when its animation ends, or when dismissed", async () => {
  const p = await openApp({ server: { waiting: [a, b] } });
  p.page.props.sent("amend", a);
  await p.settle();
  const line = recordedLine(p);
  line.props.onAnimationEnd({ animationName: "recorded-in" });
  await p.settle();
  assert.ok(p.recorded);
  line.props.onAnimationEnd({ animationName: "recorded-out" });
  await p.settle();
  assert.equal(p.recorded, undefined);
  p.page.props.sent("amend", a);
  await p.settle();
  let prevented = false;
  const dismiss = [...nodes(recordedLine(p))].find((v) => v.props["aria-label"] === "dismiss this confirmation");
  dismiss.props.onClick({ preventDefault: () => (prevented = true) });
  await p.settle();
  assert.equal(prevented, true);
  assert.equal(p.recorded, undefined);
  p.app.unmount();
});

test("a refusal's case takes the shown case's place on its page, and reads nothing", async () => {
  const p = await openApp({ url: "/cases/bC4yR", server: { waiting: [a, b] } });
  p.taken();
  p.log.length = 0;
  p.page.props.refused({ ...b, revision: 7, title: "Moved on" });
  await p.settle();
  assert.equal(p.page.props.data.shown.case.title, "Moved on");
  assert.deepEqual(p.taken(), []);
  assert.deepEqual(p.log, []);
  // An older one does not.
  p.page.props.refused({ ...b, revision: 2, title: "Older" });
  await p.settle();
  assert.equal(p.page.props.data.shown.case.title, "Moved on");
  p.app.unmount();
});

test("a refusal on the list pins its case to its own page, under the same filter, with no move", async () => {
  const p = await openApp({ url: "/?show=all", server: { waiting: [a, b] } });
  p.taken();
  p.log.length = 0;
  p.page.props.refused({ ...a, revision: 7, title: "Moved on" });
  await p.settle();
  assert.deepEqual(p.log, ["replace /cases/aB3xQ-moved-on?show=all"]);
  assert.deepEqual(p.entries(), [">/cases/aB3xQ-moved-on?show=all"]);
  assert.deepEqual(route(p), { page: "case", id: "aB3xQ", show: "all" });
  assert.deepEqual(p.taken(), [WAITING, OTHERS, "GET /v1/cases/aB3xQ", LISTENERS]);
  assert.equal(shown(p), "aB3xQ");
  // The inbox keeps its filter on the case's page too.
  const q = await openApp({ server: { waiting: [a, b] } });
  q.page.props.refused({ ...a, state: "answered" });
  await q.settle();
  assert.deepEqual(route(q), { page: "case", id: "aB3xQ", show: "inbox" });
  assert.deepEqual(q.entries(), [">/cases/aB3xQ-first-case"]);
  p.app.unmount();
  q.app.unmount();
});

test("a refusal on the list drops a read of the list still out", async () => {
  const p = await openApp({ server: { waiting: [a, b] } });
  let release;
  p.server.routes[WAITING] = () => new Promise((r) => (release = r));
  await event(p, "v2");
  delete p.server.routes[WAITING];
  p.page.props.refused({ ...a, revision: 7 });
  await p.settle();
  release({ status: 200, body: { list: [b] } });
  await p.settle();
  assert.deepEqual(route(p), { page: "case", id: "aB3xQ", show: "inbox" });
  assert.equal(shown(p), "aB3xQ");
  p.app.unmount();
});

test("a read of the list that lands between a refusal and the page's next draw is dropped too", async (t) => {
  t.after(() => delete globalThis.matchMedia);
  const fades = [];
  const extra = (page) => {
    page.document.startViewTransition = (update) => {
      fades.push(page.location.pathname);
      update();
    };
    globalThis.matchMedia = () => ({ matches: false });
  };
  const p = await openApp({ server: { waiting: [a, b] }, extra });
  let release;
  p.server.routes[WAITING] = () => new Promise((r) => (release = r));
  await event(p, "v2");
  delete p.server.routes[WAITING];
  p.page.props.refused({ ...a, revision: 7 });
  // The read comes back before App draws again.
  release({ status: 200, body: { list: [b] } });
  for (let i = 0; i < 50; i++) await Promise.resolve();
  await p.settle();
  assert.equal(shown(p), "aB3xQ");
  // Only the first page cross-faded: the pinned page does not.
  assert.deepEqual(fades, ["/"]);
  p.app.unmount();
});

// The ways the composer closes, as App hears them: Escape and its close
// control both close its dialog, whose close event calls onClose; a send
// calls onSent, then closes it; the back button takes its entry off, and
// the composer, with nothing written, sets nothing for back to answer.
const closes = {
  "escape or close control": async (p) => p.composer.props.onClose(),
  send: async (p) => {
    p.composer.props.onSent(kase("fG7bV", "A note", { kind: "message", state: "sent", worker: "w1" }));
    p.composer.props.onClose();
  },
  back: async (p) => {
    p.composer.props.back.current = null;
    p.history.back();
  },
};

for (const [how, close] of Object.entries(closes)) {
  test(`a refusal on the list while the composer is open stays shown once the composer closes (${how})`, async () => {
    const p = await openApp({ server: { waiting: [a, b] } });
    await p.click(link("/message?to=w1"));
    // The store moved on, and the send made before is refused with the case
    // as it is now.
    const now = { ...a, revision: 7 };
    p.server.waiting = [now, b];
    p.page.props.refused(now);
    await p.settle();
    // The case's page goes in under the composer, whose entry stays on top.
    assert.deepEqual(p.entries(), ["/", "/cases/aB3xQ-first-case", ">/cases/aB3xQ-first-case"]);
    assert.deepEqual(p.history.state, { composer: { to: "w1", re: "" }, show: "inbox" });
    assert.deepEqual(route(p), { page: "case", id: "aB3xQ", show: "inbox" });
    assert.ok(p.composer);
    p.taken();
    await close(p);
    await p.settle();
    // The page stays on the refused case: nothing is read again, the case
    // shown is the one the refusal gave, and the address is its page.
    assert.equal(p.composer, undefined);
    assert.deepEqual(route(p), { page: "case", id: "aB3xQ", show: "inbox" });
    assert.equal(p.page.props.data.shown.case.revision, 7);
    assert.deepEqual(p.taken(), []);
    assert.deepEqual(p.entries(), ["/", ">/cases/aB3xQ-first-case", "/cases/aB3xQ-first-case"]);
    assert.equal(p.location.pathname, "/cases/aB3xQ-first-case");
    // Back goes to the list the refusal moved from; forward to the case's
    // page again, under the filter it was pinned under, then to the
    // composer's entry, which opens it again, as forward onto any closed
    // composer's entry does.
    await p.back();
    assert.deepEqual(route(p), { page: "list", show: "inbox" });
    await p.forward();
    assert.deepEqual(route(p), { page: "case", id: "aB3xQ", show: "inbox" });
    assert.equal(p.composer, undefined);
    p.taken();
    await p.forward();
    assert.equal(p.composer.props.to, "w1");
    // Under the filter the page was pinned under: nothing is read again.
    assert.deepEqual(route(p), { page: "case", id: "aB3xQ", show: "inbox" });
    assert.deepEqual(p.taken(), []);
    p.app.unmount();
  });
}

test("a refusal keeps where its entry was scrolled, for a later back to it", async () => {
  const p = await openApp({ server: { waiting: [a, b] } });
  p.window.scrollY = 80;
  await p.click(link("/cases/bC4yR"));
  await p.back();
  // Back on the list, scrolled as it was; then its case is refused.
  p.page.props.refused({ ...a, revision: 7 });
  await p.settle();
  await p.forward();
  p.log.length = 0;
  await p.back();
  assert.deepEqual(route(p), { page: "case", id: "aB3xQ", show: "inbox" });
  assert.deepEqual(p.log, ["scrollTo 80"]);
  p.app.unmount();
});

test("another composer opened over a refusal's case puts no second entry on", async () => {
  const p = await openApp({ server: { waiting: [a, b] } });
  await p.click(link("/message?to=w1"));
  p.page.props.refused({ ...a, revision: 7 });
  await p.settle();
  mock.timers.tick(1000);
  await p.click(link("/message?to=w2"));
  assert.equal(p.composer.props.to, "w2");
  assert.deepEqual(p.entries(), ["/", "/cases/aB3xQ-first-case", ">/cases/aB3xQ-first-case"]);
  p.composer.props.onClose();
  await p.settle();
  assert.deepEqual(route(p), { page: "case", id: "aB3xQ", show: "inbox" });
  p.app.unmount();
});

test("back onto the address of the page a refusal pinned keeps it; back elsewhere moves", async () => {
  // Back onto the pinned page's entry, from a page moved to after, shows it
  // under the filter it was pinned under.
  const k = await openApp({ server: { waiting: [a, b] } });
  k.window.scrollY = 80;
  k.page.props.refused({ ...a, revision: 7 });
  await k.settle();
  await k.click(link("/cases/bC4yR"));
  k.log.length = 0;
  await k.back();
  assert.deepEqual(route(k), { page: "case", id: "aB3xQ", show: "inbox" });
  assert.deepEqual(k.log, ["scrollTo 80"]);
  k.app.unmount();
  // From the pinned page, back to another case's page moves to it.
  const p = await openApp({ url: "/cases/bC4yR", server: { waiting: [a, b] } });
  await p.click(link("/"));
  p.page.props.refused({ ...a, revision: 7 });
  await p.settle();
  assert.deepEqual(route(p), { page: "case", id: "aB3xQ", show: "inbox" });
  await p.back();
  assert.deepEqual(route(p), { page: "case", id: "bC4yR", show: null });
  p.app.unmount();
  // Back onto another entry at the pinned page's address, from before the
  // refusal, is a move too: that entry lists the case under its own filter.
  const o = await openApp({ url: "/cases/aB3xQ", server: { waiting: [a, b] } });
  await o.click(link("/"));
  o.page.props.refused({ ...a, revision: 7 });
  await o.settle();
  assert.deepEqual(o.entries(), ["/cases/aB3xQ", ">/cases/aB3xQ-first-case"]);
  o.taken();
  await o.back();
  assert.deepEqual(route(o), { page: "case", id: "aB3xQ", show: null });
  assert.deepEqual(o.taken(), [WAITING, OTHERS, "GET /v1/cases/aB3xQ", LISTENERS]);
  o.app.unmount();
  // Unpinned, back from a case under a filter to its page with none moves
  // too: it lists the case under its own.
  const q = await openApp({ url: "/cases/aB3xQ", server: { waiting: [a, b] } });
  await q.click(link("/cases/aB3xQ?show=all"));
  assert.deepEqual(route(q), { page: "case", id: "aB3xQ", show: "all" });
  q.taken();
  await q.back();
  assert.deepEqual(route(q), { page: "case", id: "aB3xQ", show: null });
  assert.deepEqual(q.taken(), [WAITING, OTHERS, "GET /v1/cases/aB3xQ", LISTENERS]);
  q.app.unmount();
});

test("a refusal on the list for a case it does not show leaves the page", async () => {
  const p = await openApp({ server: { waiting: [a, b] } });
  p.page.props.refused({ ...b, revision: 7 });
  await p.settle();
  assert.deepEqual(route(p), { page: "list", show: "inbox" });
  assert.deepEqual(p.entries(), [">/"]);
  p.app.unmount();
});

test("the options chip and the ? key open their dialogs", async () => {
  const p = await openApp({ server: { waiting: [a] } });
  const chip = [...nodes(p.tree)].find((v) => v.props.id === "options-open");
  chip.props.onClick();
  await p.settle();
  assert.equal(await p.key("?"), true);
  assert.deepEqual(p.log.filter((l) => l.startsWith("showModal")), ["showModal options", "showModal keys"]);
  p.app.unmount();
});

test("a code block's copy button copies through App's listener", async (t) => {
  const copied = [];
  const nav = Object.getOwnPropertyDescriptor(globalThis, "navigator");
  t.after(() => Object.defineProperty(globalThis, "navigator", nav));
  Object.defineProperty(globalThis, "navigator", { configurable: true, value: { clipboard: { writeText: async (t) => copied.push(t) } } });
  const p = await openApp({ server: { waiting: [a] } });
  const code = { textContent: "echo hi\n", parentElement: { classList: { contains: () => false } } };
  const button = element({ tagName: "BUTTON" });
  button.closer[".code"] = element({ q: { "pre > code": [code] } });
  button.closer[".copy-code"] = button;
  await p.click(button);
  assert.deepEqual(copied, ["echo hi"]);
  p.app.unmount();
});

test("a notification's click moves the page to its case", async (t) => {
  t.after(() => delete globalThis.Notification);
  const shownNotes = [];
  const extra = (page) => {
    page.window.Notification = true;
    globalThis.Notification = class {
      static permission = "granted";
      constructor(title) {
        shownNotes.push(this);
        this.title = title;
      }
      close() {}
    };
  };
  const p = await openApp({ prefs: { notify: "all" }, server: { waiting: [a, b] }, extra });
  assert.deepEqual(p.requests, [WAITING, OTHERS, LISTENERS, "GET /v1/notifications"]);
  p.server.notifications = { boot: "boot1", latest: 1, items: [{ id: 1, title: "Second case", body: "", tag: "case-bC4yR", url: "/cases/bC4yR-second-case" }] };
  await p.tick(5000);
  assert.deepEqual(shownNotes.map((n) => n.title), ["Second case"]);
  shownNotes[0].onclick();
  await p.settle();
  assert.deepEqual(route(p), { page: "case", id: "bC4yR", show: null });
  assert.deepEqual(p.entries(), ["/", ">/cases/bC4yR-second-case"]);
  p.app.unmount();
});

// A browser with BroadcastChannel and, unless `locks` is false, Web Locks:
// the channels made, the lock asked for, and how to let it go.
function tabs({ locks = true } = {}) {
  const made = [];
  const lock = { asked: [], grant: null, released: false };
  const nav = Object.getOwnPropertyDescriptor(globalThis, "navigator");
  const extra = (page) => {
    page.window.BroadcastChannel = true;
    globalThis.BroadcastChannel = class {
      constructor(name) {
        this.name = name;
        this.posted = [];
        this.closed = false;
        made.push(this);
      }
      postMessage(m) {
        this.posted.push(m);
      }
      close() {
        this.closed = true;
      }
    };
    const value = locks
      ? {
          locks: {
            request(name, fn) {
              lock.asked.push(name);
              return new Promise((resolve) => {
                lock.grant = () => Promise.resolve(fn()).then(() => ((lock.released = true), resolve()));
              });
            },
          },
        }
      : {};
    Object.defineProperty(globalThis, "navigator", { configurable: true, value });
  };
  const restore = () => {
    Object.defineProperty(globalThis, "navigator", nav);
    delete globalThis.BroadcastChannel;
  };
  return { made, lock, extra, restore };
}

test("with locks, the tab that holds the lock holds the stream and passes each version to the others", async (ctx) => {
  const t = tabs();
  ctx.after(t.restore);
  const p = await openApp({ server: { waiting: [a] }, extra: t.extra });
  assert.deepEqual(t.lock.asked, ["cases-stream"]);
  assert.equal(p.streams.length, 0);
  t.lock.grant();
  await p.settle();
  assert.equal(p.streams.length, 1);
  const channel = t.made[0];
  assert.equal(channel.name, "cases-store");
  p.taken();
  await event(p, "v2");
  assert.deepEqual(channel.posted, ["v2"]);
  assert.deepEqual(p.taken(), [WAITING, OTHERS, LISTENERS]);
  // It tells the others when its stream opens, and whether after a drop.
  p.stream.onerror();
  p.stream.onopen();
  await p.settle();
  assert.deepEqual(channel.posted.slice(1), [{ opened: true, dropped: true }]);
  p.stream.onopen();
  assert.deepEqual(channel.posted.slice(2), [{ opened: true, dropped: false }]);
  // The tick opens the holder's stream again; that open tells the others
  // nothing, and its tick read no case.
  p.taken();
  await p.tick(60000);
  assert.deepEqual(p.taken(), [LISTENERS, INDEX]);
  assert.equal(p.streams.length, 2);
  p.stream.onopen();
  assert.equal(channel.posted.length, 3);
  // A hidden tab keeps its stream.
  p.document.hidden = true;
  p.document.dispatch("visibilitychange");
  assert.equal(p.stream.closed, false);
  p.app.unmount();
  await p.settle();
  assert.equal(p.streams[0].closed, true);
  assert.equal(t.lock.released, true);
  assert.equal(channel.closed, true);
});

test("with locks, a tab without the stream reads on the holder's versions and its opens", async (ctx) => {
  const t = tabs();
  ctx.after(t.restore);
  const p = await openApp({ server: { waiting: [a] }, extra: t.extra });
  const channel = t.made[0];
  p.taken();
  channel.onmessage({ data: "v2" });
  await p.settle();
  assert.deepEqual(p.taken(), [WAITING, OTHERS, LISTENERS]);
  channel.onmessage({ data: "v2" });
  await p.settle();
  assert.deepEqual(p.taken(), []);
  channel.onmessage({ data: { opened: true, dropped: false } });
  await p.settle();
  assert.deepEqual(p.taken(), [INDEX]);
  channel.onmessage({ data: { opened: true, dropped: true } });
  await p.settle();
  assert.deepEqual(p.taken(), [INDEX, WAITING, OTHERS, LISTENERS]);
  channel.onmessage({ data: null });
  await p.settle();
  assert.deepEqual(p.taken(), [INDEX]);
  // With no stream of its own, its tick reads the store.
  await p.tick(60000);
  assert.deepEqual(p.taken(), [WAITING, OTHERS, LISTENERS, INDEX]);
  assert.equal(p.streams.length, 0);
  // The lock comes to it after the page went: it opens nothing.
  p.app.unmount();
  await t.lock.grant();
  assert.equal(p.streams.length, 0);
});

test("with a channel and no locks, only a shown tab holds a stream", async (ctx) => {
  const t = tabs({ locks: false });
  ctx.after(t.restore);
  const p = await openApp({ server: { waiting: [a] }, extra: (page) => (t.extra(page), (page.document.hidden = true)) });
  assert.equal(p.streams.length, 0);
  p.document.hidden = false;
  p.document.dispatch("visibilitychange");
  assert.equal(p.streams.length, 1);
  await event(p, "v2");
  assert.deepEqual(t.made[0].posted, ["v2"]);
  p.stream.onopen();
  assert.equal(t.made[0].posted.length, 1);
  p.app.unmount();
});
