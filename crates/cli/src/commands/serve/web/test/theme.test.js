import { test } from "node:test";
import assert from "node:assert/strict";

// theme.ts is a plain script for <head>: give it the page it expects.
const stored = new Map();
globalThis.localStorage = {
  getItem: (k) => (stored.has(k) ? stored.get(k) : null),
  setItem: (k, v) => stored.set(k, String(v)),
  removeItem: (k) => stored.delete(k),
};
const root = { dataset: {} };
// The page's click listeners, and its code blocks' wrap buttons.
const clicks = [];
const buttons = [{ attrs: {}, setAttribute(k, v) { this.attrs[k] = v; } }];
globalThis.document = {
  documentElement: root,
  addEventListener: (type, f) => type === "click" && clicks.push(f),
  querySelectorAll: (selector) => (selector === ".wrap-toggle" ? buttons : []),
};
// The pointer, which the key hints' default follows: fine unless a test
// makes it coarse.
let coarse = false;
globalThis.window = { addEventListener() {}, matchMedia: (q) => ({ matches: q === "(pointer: coarse)" && coarse }) };
await import("../static/theme.ts");
const prefs = window.casesPrefs;

function fresh() {
  stored.clear();
  prefs.apply();
}

test("with no face chosen, each look reads in its own face and none is set", () => {
  fresh();
  assert.equal(prefs.read("face"), "sans");
  assert.equal(root.dataset.face, undefined);
  prefs.write("look", "mono");
  prefs.apply();
  assert.equal(root.dataset.look, "mono");
  assert.equal(prefs.read("face"), "mono");
  assert.equal(root.dataset.face, undefined);
});

test("daylight is the default, stored as nothing; mono and keel are stored", () => {
  fresh();
  assert.equal(prefs.read("look"), "daylight");
  assert.equal(root.dataset.look, undefined);
  prefs.write("look", "mono");
  assert.equal(stored.get("cases.look"), "mono");
  prefs.write("look", "keel");
  assert.equal(stored.get("cases.look"), "keel");
  prefs.write("look", "daylight");
  assert.equal(stored.has("cases.look"), false);
  // A daylight stored while it was not the default still reads as daylight.
  stored.set("cases.look", "daylight");
  prefs.apply();
  assert.equal(prefs.read("look"), "daylight");
  assert.equal(root.dataset.look, undefined);
  for (const look of ["mono", "keel"]) {
    stored.set("cases.look", look);
    prefs.apply();
    assert.equal(prefs.read("look"), look);
    assert.equal(root.dataset.look, look);
  }
});

test("a face chosen is kept, and set, even when it is the look's own", () => {
  fresh();
  prefs.write("look", "mono");
  prefs.write("face", "mono");
  prefs.apply();
  assert.equal(prefs.read("face"), "mono");
  assert.equal(root.dataset.face, "mono");
  prefs.write("face", "sans");
  prefs.apply();
  assert.equal(root.dataset.face, "sans");
  prefs.write("look", "daylight");
  prefs.apply();
  assert.equal(root.dataset.look, undefined);
  assert.equal(prefs.read("face"), "sans");
  assert.equal(root.dataset.face, "sans");
});

test("reset goes back to daylight and its face", () => {
  fresh();
  prefs.write("look", "mono");
  prefs.write("face", "serif");
  for (const option of prefs.options) prefs.write(option.name, option.values[0]);
  prefs.apply();
  assert.equal(root.dataset.face, undefined);
  assert.equal(root.dataset.look, undefined);
  assert.equal(prefs.read("look"), "daylight");
  assert.equal(prefs.read("face"), "sans");
});

test("the key hints show by default with a fine pointer and hide with a coarse one, and set nothing", () => {
  fresh();
  assert.equal(prefs.read("hints"), "show");
  assert.equal(root.dataset.hints, undefined);
  coarse = true;
  try {
    assert.equal(prefs.read("hints"), "hide");
    assert.equal(root.dataset.hints, undefined);
  } finally {
    coarse = false;
  }
});

test("key hints chosen are kept and set, whatever the pointer", () => {
  for (const pointer of [false, true]) {
    coarse = pointer;
    try {
      for (const hints of ["show", "hide"]) {
        fresh();
        prefs.write("hints", hints);
        prefs.apply();
        assert.equal(stored.get("cases.hints"), hints);
        assert.equal(prefs.read("hints"), hints);
        assert.equal(root.dataset.hints, hints);
      }
    } finally {
      coarse = false;
    }
  }
});

test("reset goes back to the pointer's key hints", () => {
  fresh();
  prefs.write("hints", "hide");
  prefs.apply();
  for (const option of prefs.options) prefs.write(option.name, option.values[0]);
  prefs.apply();
  assert.equal(stored.has("cases.hints"), false);
  assert.equal(root.dataset.hints, undefined);
  assert.equal(prefs.read("hints"), "show");
});

test("approve is not pre-selected by default; on is stored and set", () => {
  fresh();
  assert.equal(prefs.read("approve"), "off");
  assert.equal(root.dataset.approve, undefined);
  prefs.write("approve", "on");
  prefs.apply();
  assert.equal(prefs.read("approve"), "on");
  assert.equal(root.dataset.approve, "on");
  prefs.write("approve", "off");
  prefs.apply();
  assert.equal(stored.has("cases.approve"), false);
  assert.equal(root.dataset.approve, undefined);
});

test("long lines scroll by default; a block's wrap button wraps them in every block, and back", () => {
  fresh();
  assert.equal(prefs.read("wrap"), "scroll");
  assert.equal(root.dataset.wrap, undefined);
  assert.equal(buttons[0].attrs["aria-pressed"], "false");
  // A click on the button, or on what is inside it.
  const press = () => clicks.forEach((f) => f({ target: { closest: (s) => (s === ".wrap-toggle" ? buttons[0] : null) } }));
  press();
  assert.equal(stored.get("cases.wrap"), "wrap");
  assert.equal(root.dataset.wrap, "wrap");
  assert.equal(buttons[0].attrs["aria-pressed"], "true");
  press();
  assert.equal(stored.has("cases.wrap"), false);
  assert.equal(root.dataset.wrap, undefined);
  assert.equal(buttons[0].attrs["aria-pressed"], "false");
  // A click elsewhere changes nothing.
  clicks.forEach((f) => f({ target: { closest: () => null } }));
  assert.equal(root.dataset.wrap, undefined);
});
