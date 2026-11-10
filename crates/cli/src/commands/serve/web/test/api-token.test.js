// api.ts before it holds a token: node runs each test file in a process of
// its own, so the module here starts as a page does, with no token read.
// The tests run in order, and no token is read until the last.
import { test } from "node:test";
import assert from "node:assert/strict";
import * as api from "../static/api.ts";

// A fetch that records each call, as "METHOD url", and answers it by `route`.
function serve(t, route) {
  const was = globalThis.fetch;
  t.after(() => { globalThis.fetch = was; });
  const calls = [];
  globalThis.fetch = async (url, init) => {
    calls.push(`${init && init.method ? init.method : "GET"} ${url}`);
    return route(url, init);
  };
  return calls;
}

test("a send whose first token read cannot reach serve says it was not recorded, and is not sent", async (t) => {
  const calls = serve(t, () => { throw new TypeError("Failed to fetch"); });
  const sent = await api.send("abc", "answer", { revision: 2 });
  assert.doesNotMatch(sent.refusal, /may or may not/);
  assert.match(sent.refusal, /not recorded/);
  assert.deepEqual(calls, ["GET /v1"]);
});

test("a message whose first token read serve refuses says it was not sent, and is not sent", async (t) => {
  const calls = serve(t, () => new Response("unavailable", { status: 503 }));
  const sent = await api.sendMessage({ body: "hi" });
  assert.doesNotMatch(sent.refusal, /may or may not/);
  assert.match(sent.refusal, /not sent/);
  assert.deepEqual(calls, ["GET /v1"]);
});

test("the index serve refuses throws with its status, and gives no token", async (t) => {
  const calls = serve(t, (url) => {
    if (url === "/v1") return new Response(JSON.stringify({ csrf: "refused", version: "0.0.0+aa" }), { status: 500 });
    throw new Error("a write sent with no token");
  });
  await assert.rejects(api.index(), { message: "GET /v1: 500" });
  // With no token, the next write reads it first, and goes no further.
  await api.send("abc", "answer", { revision: 2 });
  assert.deepEqual(calls, ["GET /v1", "GET /v1"]);
});

test("a first write reads the token, sends it, and keeps it for the next", async (t) => {
  const tokens = [];
  const calls = serve(t, (url, init) => {
    if (url === "/v1") return new Response(JSON.stringify({ csrf: "first", version: "0.0.0+aa" }), { status: 200 });
    tokens.push(init.headers["X-Cases-CSRF"]);
    return new Response(JSON.stringify({ id: "abc", revision: 3 }), { status: 200 });
  });
  await api.send("abc", "answer", { revision: 2 });
  await api.send("abc", "park", { revision: 3 });
  assert.deepEqual(tokens, ["first", "first"]);
  assert.deepEqual(calls, ["GET /v1", "POST /v1/cases/abc/answer", "POST /v1/cases/abc/park"]);
});
