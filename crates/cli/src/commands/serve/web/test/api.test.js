import { test } from "node:test";
import assert from "node:assert/strict";
import { listeners, send, upgrade } from "../static/api.ts";

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
