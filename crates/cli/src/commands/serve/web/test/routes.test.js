import { test } from "node:test";
import assert from "node:assert/strict";
import { compose, filter, moved, query, same, under, where } from "../static/routes.ts";

const url = (address) => new URL(address, "http://localhost");

test("the list's address names its filter, and the inbox is the default", () => {
  assert.deepEqual(where(url("/")), { page: "list", show: "inbox" });
  for (const show of ["inflight", "done", "all"]) {
    assert.deepEqual(where(url(`/?show=${show}`)), { page: "list", show });
  }
  assert.deepEqual(where(url("/?show=inbox")), { page: "list", show: "inbox" });
  assert.deepEqual(where(url("/?show=nonsense")), { page: "list", show: "inbox" });
});

test("a case's address keeps the filter, or has none and lists the case under its own", () => {
  assert.deepEqual(where(url("/cases/UkLWZ-pin-bun?show=done")), { page: "case", id: "UkLWZ", show: "done" });
  assert.deepEqual(where(url("/cases/UkLWZ")), { page: "case", id: "UkLWZ", show: null });
  assert.deepEqual(where(url("/cases/%FF")), { page: "case", id: "%FF", show: null });
  const state = (s) => under({ state: s });
  assert.deepEqual(["open", "parked", "answered", "pickedup", "closed", "withdrawn", "sent"].map(state), ["inbox", "inbox", "inflight", "inflight", "done", "done", "inflight"]);
});

test("each filter's address reads back as that filter", () => {
  for (const show of ["inbox", "inflight", "done", "all"]) {
    assert.equal(filter(show), show);
    assert.equal(where(url(`/${query(show)}`)).show, show);
    assert.equal(where(url(`/cases/aB3xQ-title${query(show)}`)).show, show === "inbox" ? null : show);
  }
  assert.equal(query("inbox"), "");
  assert.equal(filter(null), null);
});

test("/done is the done filter, or the in-flight one it had, at the list's address", () => {
  const alias = (address) => {
    const u = url(address);
    return [where(u), moved(u)];
  };
  assert.deepEqual(alias("/done"), [{ page: "list", show: "done" }, "/?show=done"]);
  assert.deepEqual(alias("/done?show=inflight"), [{ page: "list", show: "inflight" }, "/?show=inflight"]);
  assert.deepEqual(alias("/done?show=closed-today"), [{ page: "list", show: "done" }, "/?show=done"]);
  assert.deepEqual(alias("/done?show=all#x"), [{ page: "list", show: "done" }, "/?show=done#x"]);
  assert.equal(moved(url("/?show=done")), null);
  assert.equal(moved(url("/cases/aB3xQ")), null);
});

test("paths that are none of the app's pages are none, and routes compare by page, case and filter", () => {
  for (const path of ["/inbox", "/cases", "/cases/", "/cases/a/b", "/done/x", "/donex"]) assert.equal(where(url(path)), null, path);
  assert.ok(same(where(url("/?show=done")), where(url("/done"))));
  assert.ok(!same(where(url("/")), where(url("/?show=all"))));
  assert.ok(!same(where(url("/cases/a")), where(url("/cases/a?show=all"))));
});

test("the composer's address says who a message goes to and what it is about, and reads back", () => {
  assert.deepEqual(where(url("/message")), { page: "compose", show: null, to: "", re: "" });
  assert.deepEqual(where(url("/message?to=bun-pins&re=UkLWZ-pin-bun&show=inflight")), { page: "compose", show: "inflight", to: "bun-pins", re: "UkLWZ" });
  assert.equal(compose(), "/message");
  assert.equal(compose("bun pins", "UkLWZ"), "/message?to=bun+pins&re=UkLWZ");
  assert.deepEqual(where(url(compose("bun pins", "UkLWZ"))), { page: "compose", show: null, to: "bun pins", re: "UkLWZ" });
  assert.ok(same(where(url("/message?to=a")), where(url("/message?to=a"))));
  assert.ok(!same(where(url("/message?to=a")), where(url("/message?to=b"))));
});
