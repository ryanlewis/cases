import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { advisories, bundled, inventoryProblems, judge, locked, satisfies, vendored } from "../advisories.js";

const web = new URL("../", import.meta.url);
const read = (path) => readFileSync(new URL(path, web), "utf8");

const BANNER = `x();
/*! Bundled license information:

dompurify/dist/purify.es.mjs:
  (*! @license DOMPurify 3.4.12 | (c) Cure53 *)

@scope/pkg/index.js:
  (** @license *)
*/`;

test("the packages a bundle embeds are read from its licence banners", () => {
  assert.deepEqual(bundled(BANNER), ["dompurify", "@scope/pkg"]);
  assert.deepEqual(bundled("no banner here"), []);
  assert.deepEqual(bundled(BANNER.replace(/\n/g, "\r\n")), ["dompurify", "@scope/pkg"], "CRLF");
  assert.deepEqual(bundled("/*! Bundled license information:\n\nkhroma:\n  (*x*)\n*/"), ["khroma"], "no path");
});

test("each vendored release is read from the README's table, once", () => {
  const releases = vendored(read("static/vendor/README.md"));
  const names = releases.map((r) => `${r.package}@${r.version}`);
  assert.equal(new Set(names).size, names.length);
  for (const name of ["preact", "htm", "marked", "mermaid", "@fontsource-variable/inter"]) {
    assert.ok(releases.some((r) => r.package === name), name);
  }
});

test("the lockfile's packages are read with their versions", () => {
  const lock = { packages: { "": { name: "root" }, "node_modules/a": { version: "1.0.0" }, "node_modules/b/node_modules/@s/c": { version: "2.0.0" }, "node_modules/d": { link: true } } };
  assert.deepEqual(locked(lock), [
    { package: "a", version: "1.0.0", where: "npm" },
    { package: "@s/c", version: "2.0.0", where: "npm" },
  ]);
  assert.deepEqual(locked({ packages: { "packages/lib": { name: "lib", version: "1.0.0" } } }), [{ package: "lib", version: "1.0.0", where: "npm" }]);
  assert.throws(() => locked({ packages: { "packages/some-lib": { version: "1.0.0" } } }), /no name/);
});

test("the inventory fails when a banner is unlisted or its version has moved", () => {
  const inventory = { bundled: [{ package: "dompurify", version: "3.4.12", file: "f.mjs", marker: "@license DOMPurify 3.4.12 " }] };
  assert.equal(inventoryProblems(inventory, new Map([["f.mjs", BANNER]])).length, 1, "@scope/pkg is not listed");
  const moved = BANNER.replace("3.4.12", "3.4.13").replace("@scope/pkg/index.js:", "");
  assert.match(inventoryProblems(inventory, new Map([["f.mjs", moved]]))[0], /does not contain/);
  assert.match(inventoryProblems(inventory, new Map())[0], /not vendored/);
});

test("the checked-in inventory matches the vendored files", () => {
  const inventory = JSON.parse(read("vendor-inventory.json"));
  const files = new Map(inventory.bundled.map((e) => [e.file, read(e.file)]));
  assert.deepEqual(inventoryProblems(inventory, files), []);
});

test("advisory ranges are read as npm writes them", () => {
  assert.ok(satisfies("3.4.12", "<=3.4.16"));
  assert.ok(!satisfies("3.4.17", "<=3.4.16"));
  assert.ok(satisfies("3.4.12", ">=3.4.4 <=3.4.16"));
  assert.ok(!satisfies("3.4.3", ">=3.4.4 <=3.4.16"));
  assert.ok(satisfies("4.17.23", ">=4.0.0 <=4.17.23"));
  assert.ok(satisfies("1.0.0", "<0.5.0 || >=1.0.0 <1.1.0"));
  assert.ok(!satisfies("4.18.1", "<=4.17.23"));
  assert.ok(satisfies("2.0.0-rc.1", "<2.0.0"));
  assert.ok(satisfies("9.9.9", "*"));
  assert.ok(satisfies("0.5.0", "< 1.0.0"), "an operator apart from its version");
  assert.ok(satisfies("1.0.0+b.1", "<=1.0.0"), "build metadata is ignored");
});

test("a prerelease is ordered by its identifiers, as semver orders them", () => {
  assert.ok(!satisfies("1.0.0-beta-2", "=1.0.0-beta-3"));
  assert.ok(satisfies("1.0.0-rc.9", "<1.0.0-rc.10"));
  assert.ok(!satisfies("1.0.0-rc.10", "<1.0.0-rc.9"));
  assert.ok(satisfies("1.0.0-alpha", "<1.0.0-alpha.1"));
  assert.ok(satisfies("1.0.0-1", "<1.0.0-alpha"));
});

test("a range the script cannot read throws, so no advisory is dropped unread", () => {
  for (const range of ["^1.0.0", "~1.2.0", "1.x", "1.0.0 - 2.0.0", "<"]) {
    assert.throws(() => satisfies("1.5.0", range), /unread range/, range);
  }
});

test("only the advisories whose range holds the version are hits", async () => {
  const post = async () => ({ ok: true, json: async () => ({ x: [{ url: "https://github.com/advisories/GHSA-aaaa-bbbb-cccc", severity: "high", title: "t", vulnerable_versions: "<2.0.0" }] }) });
  const hits = await advisories([{ package: "x", version: "1.0.0", where: "npm" }, { package: "x", version: "2.0.0", where: "vendored" }], post);
  assert.deepEqual(hits.map((h) => [h.id, h.version, h.where]), [["GHSA-aaaa-bbbb-cccc", "1.0.0", "npm"]]);
  const bare = await advisories([{ package: "x", version: "1.0.0", where: "npm" }], async () => ({ ok: true, json: async () => ({ x: [{ id: 42, severity: "low", title: "t", vulnerable_versions: "*" }] }) }));
  assert.equal(bare[0].id, "npm-42", "an advisory without a url is named by its npm id");
  await assert.rejects(advisories([], async () => ({ ok: false, status: 503, statusText: "down" })), /503/);
});

test("an exception covers one advisory at one version in one place, and one unused is stale", () => {
  const hit = { id: "GHSA-1", package: "p", version: "1.0.0", where: "vendored" };
  const exception = { ...hit, reason: "r" };
  assert.deepEqual(judge([hit], [exception]), { open: [], stale: [] });
  assert.deepEqual(judge([{ ...hit, where: "npm" }], [exception]).open.length, 1);
  assert.deepEqual(judge([{ ...hit, version: "1.0.1" }], [exception]).open.length, 1);
  assert.deepEqual(judge([], [exception]).stale, [exception]);
});

test("each exception gives its reason", () => {
  for (const e of JSON.parse(read("advisory-exceptions.json")).exceptions) {
    assert.match(e.id, /^GHSA-/);
    assert.ok(["vendored", "npm"].includes(e.where), e.id);
    assert.ok(e.reason.length > 40, e.id);
  }
});
