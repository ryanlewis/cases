// The case view's send (IN-09): one write at a time, and the form follows
// the newest case a write or a refusal gave.
import { test, mock } from "node:test";
import assert from "node:assert/strict";
import { sendOnce } from "../static/case.ts";

const drawn = (revision) => ({ revision, amendSeq: 0, state: "open", seq: 1 });
const settle = async () => {
  for (let i = 0; i < 20; i++) await new Promise((r) => setImmediate(r));
};

// Stands in for serve: the token at /v1, and each POST held until the test
// answers it with `answer(i, status, body)`.
function serve(t) {
  const was = globalThis.fetch;
  t.after(() => {
    globalThis.fetch = was;
  });
  const posts = [];
  globalThis.fetch = (url, init = {}) => {
    if (url === "/v1") return Promise.resolve(new Response(JSON.stringify({ csrf: "tok" }), { status: 200 }));
    return new Promise((resolve) => posts.push({ url, body: JSON.parse(init.body), resolve }));
  };
  const answer = (i, status, body) => posts[i].resolve(new Response(JSON.stringify(body), { status }));
  return { posts, answer };
}

// A view's state as sendOnce sees it: the form as drawn, with setters that
// apply as preact's would, and the one in-flight guard every send shares.
function view(revision = 3) {
  const state = { drawn: drawn(revision), refusal: "", sent: [], cases: [] };
  const sending = { current: null };
  const send = (what, body = {}) =>
    sendOnce({
      id: "c1Ab2",
      what,
      body,
      drawn: state.drawn,
      setDrawn: (next) => (state.drawn = typeof next === "function" ? next(state.drawn) : next),
      setRefusal: (r) => (state.refusal = r),
      sending,
      onSent: (w, after) => state.sent.push([w, after.revision]),
      onCase: (c) => state.cases.push(c.revision),
    });
  return { state, send, sending };
}

const kase = (revision) => ({ id: "c1Ab2", kind: "decision", state: "open", revision, amend_seq: 0, last_seq: revision });

test("a send slower than the guard's 10 s does not release the guard of the send after it", async (t) => {
  mock.timers.enable({ apis: ["setTimeout"] });
  t.after(() => mock.timers.reset());
  const { posts, answer } = serve(t);
  const { send } = view();
  const a = send("park");
  await settle();
  assert.equal(posts.length, 1);
  // A is out for more than 10 s: the guard lets B go.
  mock.timers.tick(10000);
  const b = send("park");
  await settle();
  assert.equal(posts.length, 2);
  // A comes back while B is still out: a third send must wait for B.
  answer(0, 200, kase(4));
  await a;
  send("park");
  await settle();
  assert.equal(posts.length, 2, "a third write went out while B was");
  answer(1, 200, kase(5));
  await b;
  // Once B is back, the next send goes.
  const c = send("park");
  await settle();
  assert.equal(posts.length, 3);
  answer(2, 200, kase(6));
  await c;
});

test("a slow refusal does not roll the form back past the case a later write left", async (t) => {
  mock.timers.enable({ apis: ["setTimeout"] });
  t.after(() => mock.timers.reset());
  const { posts, answer } = serve(t);
  const { state, send } = view(3);
  const a = send("park");
  await settle();
  mock.timers.tick(10000);
  const b = send("park");
  await settle();
  // B lands first, at revision 5.
  answer(1, 200, kase(5));
  await b;
  assert.equal(state.drawn.revision, 5);
  // Then A's refusal comes back with the case as it was at revision 4.
  answer(0, 409, { error: { category: "stale", message: "stale", case: kase(4) } });
  await a;
  assert.equal(state.drawn.revision, 5);
  // The refusal is still said, and the case it gave handed on: the page
  // keeps the newer case it shows (withRefused, page.test.js).
  assert.match(state.refusal, /changed after it was drawn/);
  assert.deepEqual(state.cases, [4]);
});

test("a refusal with a newer case moves the form on to it", async (t) => {
  const { posts, answer } = serve(t);
  const { state, send } = view(3);
  const a = send("park");
  await settle();
  answer(0, 409, { error: { category: "stale", message: "stale", case: kase(7) } });
  await a;
  assert.equal(state.drawn.revision, 7);
  assert.equal(posts.length, 1);
});
