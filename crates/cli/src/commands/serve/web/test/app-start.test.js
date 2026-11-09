// How start() (static/app.ts) mounts App on a page that has no build
// written into it: with the build serve says it is. app-page.js loads
// app.ts on the page set here, before any test runs; app-load.test.js
// pins a page that says its build.
import { test } from "node:test";
import assert from "node:assert/strict";

globalThis.firstPage = { meta: null, server: { build: "b7" } };
const { INDEX, first } = await import("./app-page.js");

test("start() on a page with no build in it asks serve for its build", () => {
  assert.equal(first.build, "b7");
  assert.deepEqual(first.requests, [INDEX]);
  assert.equal(first.scrollRestoration, "manual");
});
