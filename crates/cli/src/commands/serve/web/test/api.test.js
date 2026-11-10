import { test } from "node:test";
import assert from "node:assert/strict";
import { listeners, send, upgrade } from "../static/api.ts";
import * as api from "../static/api.ts";

test("a page reloads for another build of serve, or says so when something typed would be lost", () => {
  assert.equal(upgrade("0.0.0+aa", "0.0.0+aa", false), "same");
  assert.equal(upgrade("0.0.0+aa", "0.0.0+aa", true), "same");
  assert.equal(upgrade("0.0.0+bb", "0.0.0+aa", false), "reload");
  assert.equal(upgrade("0.0.0+bb", "0.0.0+aa", true), "say");
});

test("who is listening is not known, not no one, when serve cannot be reached or refuses the read", async (t) => {
  const was = globalThis.fetch;
  t.after(() => { globalThis.fetch = was; });
  const list = [{ worker: "w", labels: [], since: "2026-09-26T12:00:00Z", heard: "2026-09-26T12:00:00Z" }];
  globalThis.fetch = async () => new Response(JSON.stringify({ list }), { status: 200 });
  assert.deepEqual(await listeners(), list);
  globalThis.fetch = async () => new Response(JSON.stringify({ list: [] }), { status: 200 });
  assert.deepEqual(await listeners(), []);
  globalThis.fetch = async () => new Response(JSON.stringify({ error: "store", message: "cannot read" }), { status: 500 });
  assert.equal(await listeners(), null);
  globalThis.fetch = async () => { throw new TypeError("Failed to fetch"); };
  assert.equal(await listeners(), null);
});

// A fetch that answers the index, then each POST and read by `route`.
function serve(t, route) {
  const was = globalThis.fetch;
  t.after(() => { globalThis.fetch = was; });
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push(`${init && init.method ? init.method : "GET"} ${url}`);
    if (url === "/v1") return new Response(JSON.stringify({ csrf: "tok", build: "b" }), { status: 200 });
    return route(url, init);
  };
  return calls;
}

const mine = { id: "abc", revision: 3, state: "answered" };

test("a send that fails after the write was recorded says the outcome is unknown, gives no case to take a revision from, and a second send still carries the first revision", async (t) => {
  const bodies = [];
  const calls = serve(t, (url, init) => {
    if (init && init.method === "POST") {
      bodies.push(JSON.parse(init.body));
      throw new TypeError("Failed to fetch");
    }
    throw new Error("unexpected read");
  });
  const draft = { revision: 2, reason: "done" };
  const first = await send("abc", "withdraw", draft);
  assert.equal(first.after, undefined);
  assert.match(first.refusal, /may or may not have been recorded/);
  assert.doesNotMatch(first.refusal, /nothing was sent/);
  assert.equal(first.current, undefined, "a case would give the next send its revision");
  await send("abc", "withdraw", draft);
  assert.deepEqual(bodies.map((b) => b.revision), [2, 2], "serve refuses the second as stale if the first was recorded");
  assert.deepEqual(calls.filter((c) => c.startsWith("GET /v1/")), []);
});

test("a send that cannot reach serve at all says the same, without the case", async (t) => {
  const calls = serve(t, () => { throw new TypeError("Failed to fetch"); });
  const sent = await send("abc", "answer", { revision: 2 });
  assert.match(sent.refusal, /may or may not have been recorded/);
  assert.equal(sent.current, undefined);
  assert.equal(calls.filter((c) => c.startsWith("POST")).length, 1);
});

test("a send refused for a token from before serve restarted is sent once more", async (t) => {
  let posts = 0;
  const calls = serve(t, (url, init) => {
    if (init && init.method === "POST") {
      posts += 1;
      return posts === 1
        ? new Response(JSON.stringify({ error: { category: "csrf", message: "bad token" } }), { status: 403 })
        : new Response(JSON.stringify(mine), { status: 200 });
    }
    throw new Error("unexpected read");
  });
  const sent = await send("abc", "answer", { revision: 2 });
  assert.deepEqual(sent.after, mine);
  assert.equal(posts, 2);
  assert.ok(calls.filter((c) => c === "GET /v1").length >= 1, "the token is read again");
});

// A fetch where every POST is refused for an old token, and the token is
// read again by `reread`, once serve has been reached for it before.
function restarted(t, reread, post = () => new Response(JSON.stringify({ error: { category: "csrf", message: "bad token" } }), { status: 403 })) {
  const was = globalThis.fetch;
  t.after(() => { globalThis.fetch = was; });
  const calls = [];
  globalThis.fetch = async (url, init) => {
    const method = init && init.method ? init.method : "GET";
    calls.push(`${method} ${url}`);
    if (url === "/v1") return calls.filter((c) => c === "GET /v1").length === 1 ? new Response(JSON.stringify({ csrf: "tok", build: "b" }), { status: 200 }) : reread();
    return post(calls.filter((c) => c.startsWith("POST")).length);
  };
  return calls;
}

test("a send refused for an old token, whose token cannot be read again, says it was not recorded", async (t) => {
  const calls = restarted(t, () => { throw new TypeError("Failed to fetch"); });
  await api.index();
  const sent = await send("abc", "answer", { revision: 2 });
  assert.doesNotMatch(sent.refusal, /may or may not/);
  assert.match(sent.refusal, /could not be reached/);
  assert.match(sent.refusal, /not recorded/);
  assert.equal(sent.current, undefined);
  assert.equal(calls.filter((c) => c.startsWith("POST")).length, 1, "the write is not sent again");
});

test("a message refused for an old token, whose token cannot be read again, says it was not sent", async (t) => {
  restarted(t, () => { throw new TypeError("Failed to fetch"); });
  await api.index();
  const sent = await api.sendMessage({ body: "hi" });
  assert.doesNotMatch(sent.refusal, /may or may not/);
  assert.match(sent.refusal, /not sent/);
});

test("a send whose token is read again, but whose second send cannot reach serve, says the outcome is unknown", async (t) => {
  const refused = new Response(JSON.stringify({ error: { category: "csrf", message: "bad token" } }), { status: 403 });
  const calls = restarted(
    t,
    () => new Response(JSON.stringify({ csrf: "new", build: "b" }), { status: 200 }),
    (n) => { if (n === 1) return refused; throw new TypeError("Failed to fetch"); },
  );
  await api.index();
  const sent = await send("abc", "answer", { revision: 2 });
  assert.match(sent.refusal, /may or may not have been recorded/);
  assert.equal(calls.filter((c) => c.startsWith("POST")).length, 2);
});

// A case as serve sends it with an event stored with no data.
const served = (revision = 2) => ({
  id: "c1Ab2", kind: "decision", state: "open", title: "T", revision, amend_seq: 0, last_seq: 2,
  events: [{ seq: 1, author: "agent", event: "open", file: "", data: null }, { seq: 2, author: "human", event: "answer", file: "", data: { choice: 1 } }],
});
const datas = (c) => c.events.map((e) => e.data);

test("a case read, written or refused comes out with empty data on each event serve sent with none", async (t) => {
  const was = globalThis.fetch;
  t.after(() => { globalThis.fetch = was; });
  let reply;
  globalThis.fetch = async (url) => (url === "/v1" ? new Response(JSON.stringify({ csrf: "tok" }), { status: 200 }) : reply());
  reply = () => new Response(JSON.stringify({ list: [served()] }), { status: 200 });
  assert.deepEqual((await api.list(["open"])).map(datas), [[{}, { choice: 1 }]]);
  reply = () => new Response(JSON.stringify(served()), { status: 200 });
  assert.deepEqual(datas((await api.one("c1Ab2")).body), [{}, { choice: 1 }]);
  assert.deepEqual(datas((await api.send("c1Ab2", "park", { revision: 2 })).after), [{}, { choice: 1 }]);
  reply = () => new Response(JSON.stringify({ error: { category: "stale", message: "stale", case: served(3) } }), { status: 409 });
  assert.deepEqual(datas((await api.send("c1Ab2", "park", { revision: 2 })).current), [{}, { choice: 1 }]);
  // A refusal without a case, and a case without events, are as they were.
  reply = () => new Response(JSON.stringify({ error: { category: "store", message: "no" } }), { status: 500 });
  assert.equal((await api.send("c1Ab2", "park", { revision: 2 })).current, undefined);
  reply = () => new Response(JSON.stringify({ ...served(), events: undefined }), { status: 200 });
  assert.equal((await api.one("c1Ab2")).body.events, undefined);
});

// A fetch that records each request, as [url, init], and answers it by
// `route`; the index gives the token `csrf`.
function recorded(t, route, csrf = "tok") {
  const was = globalThis.fetch;
  t.after(() => { globalThis.fetch = was; });
  const requests = [];
  globalThis.fetch = async (url, init) => {
    requests.push([url, init]);
    if (url === "/v1") return new Response(JSON.stringify({ csrf, version: "0.0.0+aa" }), { status: 200 });
    return route(url, init);
  };
  return requests;
}

const json = (body, status = 200) => new Response(JSON.stringify(body), { status });

test("each read asks for its route under /v1, and never from the cache", async (t) => {
  const requests = recorded(t, (url) => (url.startsWith("/v1/cases?") ? json({ list: [] }) : json({ list: [], id: "a/b" })));
  assert.deepEqual(await api.index(), { csrf: "tok", version: "0.0.0+aa" });
  await api.list(["open", "parked"]);
  await api.one("a/b");
  await api.notifications(null);
  await api.notifications(0);
  await api.notifications(7);
  assert.deepEqual(requests.map(([url]) => url), [
    "/v1",
    "/v1/cases?state=open%2Cparked",
    "/v1/cases/a%2Fb",
    "/v1/notifications",
    "/v1/notifications?after=0",
    "/v1/notifications?after=7",
  ]);
  for (const [url, init] of requests) {
    assert.equal(init.cache, "no-store", url);
    assert.equal(init.method, undefined, url);
  }
});

test("a list serve refuses throws Refused, with the refusal's status and message, or the status when the body is not JSON", async (t) => {
  let reply;
  recorded(t, () => reply());
  reply = () => json({ error: { category: "store", message: "cannot read the store" } }, 500);
  await assert.rejects(api.list(["open"]), (e) => e instanceof api.Refused && e.status === 500 && e.message === "cannot read the store");
  reply = () => new Response("<html>bad gateway</html>", { status: 502 });
  await assert.rejects(api.list(["open"]), (e) => e instanceof api.Refused && e.status === 502 && e.message === "serve answered 502");
});

test("a reply whose body is not JSON has a null body, and keeps its status", async (t) => {
  let reply;
  recorded(t, () => reply());
  reply = () => new Response("not json", { status: 200 });
  assert.deepEqual(await api.notifications(null), { ok: true, status: 200, body: null });
  reply = () => new Response("", { status: 404 });
  assert.deepEqual(await api.one("gone"), { ok: false, status: 404, body: null });
  reply = () => json({ error: { category: "missing", message: "no case gone" } }, 404);
  assert.deepEqual(await api.one("gone"), { ok: false, status: 404, body: { error: { category: "missing", message: "no case gone" } } });
});

test("a write POSTs its body as JSON to the case's route, with the token the index gave", async (t) => {
  const requests = recorded(t, () => json(mine), "write-token");
  await api.index();
  const reply = await api.write("a b", "amend", { revision: 1, note: "n" });
  assert.deepEqual(reply, { ok: true, status: 200, body: mine });
  const [url, init] = requests[1];
  assert.equal(url, "/v1/cases/a%20b/amend");
  assert.deepEqual(init, {
    method: "POST",
    headers: { "Content-Type": "application/json", "X-Cases-CSRF": "write-token" },
    body: JSON.stringify({ revision: 1, note: "n" }),
    cache: "no-store",
  });
});

test("a write refused for an old token is sent again with the token read again, and only then", async (t) => {
  let token = "old";
  const sentWith = [];
  let reply;
  let reads = 0;
  const was = globalThis.fetch;
  t.after(() => { globalThis.fetch = was; });
  globalThis.fetch = async (url, init) => {
    if (url === "/v1") {
      reads += 1;
      return json({ csrf: token, version: "0.0.0+aa" });
    }
    sentWith.push(init.headers["X-Cases-CSRF"]);
    return reply(sentWith.length);
  };
  await api.index();
  token = "new";
  reply = (n) => (n === 1 ? json({ error: { category: "csrf", message: "bad token" } }, 403) : json(mine));
  assert.deepEqual((await api.send("abc", "answer", { revision: 2 })).after, mine);
  assert.deepEqual(sentWith, ["old", "new"]);
  assert.equal(reads, 2);
  // A 403 for any other reason, or a csrf refusal of another status, is
  // not sent again, and the token is not read again for it.
  sentWith.length = 0;
  reply = () => json({ error: { category: "request", message: "a write from another site" } }, 403);
  assert.equal((await api.send("abc", "answer", { revision: 2 })).refusal, "a write from another site");
  reply = () => json({ error: { category: "csrf", message: "odd" } }, 400);
  assert.equal((await api.send("abc", "answer", { revision: 2 })).refusal, "odd");
  assert.deepEqual(sentWith, ["new", "new"]);
  assert.equal(reads, 2);
});

test("a send refused as stale says so plainly; any other refusal says what serve said, or its status", async (t) => {
  let reply;
  recorded(t, () => reply());
  await api.index();
  reply = () => json({ error: { category: "stale", message: "revision 2 is not 3" } }, 409);
  const stale = await send("abc", "answer", { revision: 2 });
  assert.equal(stale.refusal, "this case changed after it was drawn, so this was not recorded. check the thread before sending again.");
  assert.equal(stale.current, undefined);
  reply = () => json({ error: { category: "invalid", message: "choice 9 is not an option" } }, 400);
  assert.deepEqual(await send("abc", "answer", { revision: 2 }), { refusal: "choice 9 is not an option" });
  reply = () => new Response("oops", { status: 500 });
  assert.deepEqual(await send("abc", "answer", { revision: 2 }), { refusal: "serve answered 500" });
});

test("a message is POSTed to /v1/cases, and comes back as the case, as serve's refusal, or as maybe sent when serve cannot be reached", async (t) => {
  let reply;
  const requests = recorded(t, () => reply());
  await api.index();
  reply = () => json(served());
  const sent = await api.sendMessage({ body: "hi" });
  assert.deepEqual(datas(sent.after), [{}, { choice: 1 }]);
  assert.equal(sent.refusal, undefined);
  const [url, init] = requests.at(-1);
  assert.equal(url, "/v1/cases");
  assert.equal(init.method, "POST");
  assert.equal(init.body, JSON.stringify({ body: "hi" }));
  reply = () => json({ error: { category: "invalid", message: "a message needs a body" } }, 400);
  assert.deepEqual(await api.sendMessage({ body: "" }), { refusal: "a message needs a body" });
  reply = () => { throw new TypeError("Failed to fetch"); };
  const lost = await api.sendMessage({ body: "hi" });
  assert.equal(lost.after, undefined);
  assert.equal(lost.refusal, "serve could not be reached, so the message may or may not have been sent. look for it in flight before sending it again.");
});
