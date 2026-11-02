import { test } from "node:test";
import assert from "node:assert/strict";
import { render } from "preact-render-to-string";
import { Hint, keys, rowAfter, rowFor } from "../static/keys.ts";

test("the number keys set the approval row with the focus, else the first with no verdict", () => {
  assert.equal(rowFor([false, false, false], -1), 0);
  assert.equal(rowFor([true, false, false], -1), 1);
  assert.equal(rowFor([true, true, false], 0), 0);
  assert.equal(rowFor([false, true, false], 1), 1);
  assert.equal(rowFor([true, true, true], -1), -1);
});

test("after a verdict the focus goes to the next row with none, or stays", () => {
  assert.equal(rowAfter([true, false, false], 0), 1);
  assert.equal(rowAfter([true, true, false], 0), 2);
  assert.equal(rowAfter([true, true, true], 1), 1);
  // An earlier row with no verdict is not gone back to.
  assert.equal(rowAfter([false, true, true], 2), 2);
  assert.equal(rowAfter([false, true, true], 1), 1);
});

test("a hint is its keys' glyphs, hidden from screen readers unless it has a label", () => {
  assert.equal(render(Hint({ keys: ["j", "k"] })), '<span class="key-hint" aria-hidden="true"><kbd>j</kbd><kbd>k</kbd></span>');
  assert.equal(render(Hint({ keys: ["?"], label: "keys" })), '<span class="key-hint"><kbd>?</kbd> keys</span>');
});

// A stand-in for the page, as much of it as keys.ts reads. Each element
// answers a selector from `q`, the elements it holds for it, and logs what
// is done to it in `log`.
let log = [];
class El {
  constructor(id, props = {}) {
    Object.assign(this, { id, tagName: "DIV", dataset: {}, q: {}, within: [], closer: {}, ...props });
  }
  querySelectorAll(sel) {
    return this.q[sel] || [];
  }
  querySelector(sel) {
    return (this.q[sel] || [])[0] || null;
  }
  closest(sel) {
    return this.closer[sel] || null;
  }
  contains(el) {
    return el === this || this.within.includes(el);
  }
  click() {
    log.push(`click ${this.id}`);
    if (this.type === "radio") this.checked = true;
  }
  focus() {
    log.push(`focus ${this.id}`);
    page.activeElement = this;
  }
}

let page;
// Sets the page up with `form` as the open case's answer form (shown unless
// `hidden`), `cards` in the list, a dialog open if `open`, and `dialog` as
// the keys dialog, and listens on it. Gives a function that presses a key
// and says whether the page kept it from the browser.
function setup({ form = null, hidden = false, cards = [], open = false, dialog = null } = {}) {
  log = [];
  let listener;
  page = {
    activeElement: null,
    addEventListener: (type, f) => {
      assert.equal(type, "keydown");
      listener = f;
    },
    querySelector: (sel) => {
      if (sel === "dialog[open]") return open ? new El("open dialog") : null;
      if (sel === '#case form.respond[data-write="answer"]') return form;
      assert.fail(sel);
    },
    querySelectorAll: (sel) => {
      assert.equal(sel, "#inbox a.card");
      return cards;
    },
  };
  if (form) form.getClientRects = () => (hidden ? [] : [{}]);
  globalThis.document = page;
  keys(() => dialog);
  return (key, init = {}) => {
    const e = { key, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, repeat: false, isComposing: false, defaultPrevented: false, target: new El("body", { tagName: "BODY" }), ...init };
    e.preventDefault = () => (e.prevented = true);
    listener(e);
    return !!e.prevented;
  };
}

const radio = (id, checked = false) => new El(id, { tagName: "INPUT", type: "radio", checked });
// A form whose one set of choices is `radios`, with a note and a send button.
function choiceForm(radios = [radio("r1"), radio("r2")], extra = {}) {
  const form = new El("form", { tagName: "FORM", ...extra });
  form.q['input[type="radio"]'] = radios;
  form.q['textarea[name="note"]'] = [new El("note", { tagName: "TEXTAREA" })];
  form.q["#respond-send"] = [new El("send", { tagName: "BUTTON" })];
  form.requestSubmit = (by) => log.push(`submit by ${by && by.id}`);
  return form;
}
// An approval's form of `rows`, each `[verdicts checked, preset]`, with
// three verdicts a row.
function approvalForm(rows) {
  const form = choiceForm([]);
  form.q["fieldset.row"] = rows.map(([checked, preset], i) => {
    const radios = [0, 1, 2].map((v) => radio(`row${i}.${v + 1}`, checked === v + 1));
    return new El(`row${i}`, { tagName: "FIELDSET", dataset: preset ? { preset: "" } : {}, q: { 'input[type="radio"]': radios }, within: radios });
  });
  return form;
}

test("Cmd or Ctrl+Enter sends the open case's answer with its send button", () => {
  for (const mod of [{ metaKey: true }, { ctrlKey: true }]) {
    const form = choiceForm();
    const press = setup({ form });
    assert.equal(press("Enter", mod), true);
    assert.deepEqual(log, ["submit by send"]);
    // From a field of the answer's own form too.
    log = [];
    assert.equal(press("Enter", { ...mod, target: new El("f", { tagName: "TEXTAREA", form }) }), true);
    assert.deepEqual(log, ["submit by send"]);
  }
});

test("Cmd or Ctrl+Enter does nothing with Alt or Shift, no form shown, a dialog open, held, in another form, or on a link", () => {
  const form = choiceForm();
  const cases = [
    [{ form }, { metaKey: true, altKey: true }],
    [{ form }, { ctrlKey: true, shiftKey: true }],
    [{}, { metaKey: true }],
    [{ form, hidden: true }, { metaKey: true }],
    [{ form, open: true }, { metaKey: true }],
    [{ form }, { metaKey: true, repeat: true }],
    [{ form }, { metaKey: true, target: new El("other field", { form: new El("other form") }) }],
    [{ form }, { ctrlKey: true, target: new El("link", { closer: { "a[href]": new El("a") } }) }],
  ];
  for (const [page, init] of cases) {
    const press = setup(page);
    assert.equal(press("Enter", init), false, JSON.stringify(init));
    assert.deepEqual(log, []);
  }
  // A target with no `closest`, such as the document, is not a link.
  assert.equal(setup({ form })("Enter", { metaKey: true, target: {} }), true);
});

test("no key does anything once the page or the browser has taken it", () => {
  for (const init of [{ defaultPrevented: true }, { isComposing: true }]) {
    const press = setup({ form: choiceForm() });
    assert.equal(press("Enter", { metaKey: true, ...init }), false);
    assert.equal(press("1", init), false);
    assert.deepEqual(log, []);
  }
});

test("the other keys do nothing with a modifier held, in a text field, or with a dialog open", () => {
  const typed = [
    new El("t", { tagName: "TEXTAREA" }),
    new El("s", { tagName: "SELECT" }),
    new El("e", { isContentEditable: true }),
    new El("i", { tagName: "INPUT", type: "text" }),
    new El("i", { tagName: "INPUT", type: "" }),
  ];
  for (const init of [{ metaKey: true }, { ctrlKey: true }, { altKey: true }, ...typed.map((target) => ({ target }))]) {
    const press = setup({ form: choiceForm() });
    assert.equal(press("1", init), false);
    assert.deepEqual(log, []);
  }
  assert.equal(setup({ form: choiceForm(), open: true })("1"), false);
  // A choice, a button and other inputs typed into are not text fields; nor
  // is a target that is not an element, or none.
  for (const target of [radio("x"), new El("c", { tagName: "INPUT", type: "checkbox" }), new El("b", { tagName: "INPUT", type: "submit" }), new El("b", { tagName: "BUTTON" }), {}, null]) {
    assert.equal(setup({ form: choiceForm() })("1", { target }), true, target && target.id);
  }
});

test("? shows the keys dialog, when there is one to show", () => {
  const dialog = { showModal: () => log.push("shown") };
  let press = setup({ dialog });
  assert.equal(press("?"), true);
  assert.deepEqual(log, ["shown"]);
  for (const dialog of [null, {}]) {
    press = setup({ dialog });
    assert.equal(press("?"), false);
    assert.deepEqual(log, []);
  }
});

test("n goes to the note to the agent, when the open case's form has one", () => {
  const form = choiceForm();
  assert.equal(setup({ form })("n"), true);
  assert.deepEqual(log, ["focus note"]);
  form.q['textarea[name="note"]'] = [];
  assert.equal(setup({ form })("n"), false);
  assert.equal(setup({})("n"), false);
  assert.deepEqual(log, []);
});

test("j and k open the next or previous case, skipping folded cards, and stop at the ends", () => {
  const folded = new El("folded");
  const card = (id, current = false, closer = {}) => {
    const a = new El(id, { tagName: "A", closer });
    a.getAttribute = (name) => (name === "aria-current" && current ? "page" : null);
    return a;
  };
  const list = (current) => ["c0", "c1", "c2"].map((id, i) => card(id, i === current));
  const cases = [
    // [cards, key, the card opened]
    [list(-1), "j", "c0"],
    [list(-1), "k", "c2"],
    [list(0), "j", "c1"],
    [list(1), "k", "c0"],
    [list(2), "j", undefined],
    [list(0), "k", undefined],
    [[], "j", undefined],
    [[card("c0", true), card("x", false, { "details:not([open])": folded }), card("c2")], "j", "c2"],
  ];
  for (const [cards, key, opened] of cases) {
    const press = setup({ cards });
    // The key is kept from the browser whether or not a case opens.
    assert.equal(press(key), true);
    assert.deepEqual(log, opened ? [`click ${opened}`] : [], `${key} ${opened}`);
  }
});

test("a number key chooses that option of the form's one set of choices", () => {
  const press = setup({ form: choiceForm() });
  assert.equal(press("2"), true);
  assert.deepEqual(log, ["click r2", "focus r2"]);
  log = [];
  assert.equal(press("3"), false);
  assert.equal(press("0"), false);
  assert.equal(press("x"), false);
  assert.deepEqual(log, []);
  assert.equal(setup({})("1"), false);
  assert.equal(setup({ form: choiceForm(), hidden: true })("1"), false);
  // A held number key repeats the choice.
  assert.equal(setup({ form: choiceForm() })("1", { repeat: true }), true);
});

test("on an approval a number key gives the current row its verdict and moves on to the next row with none", () => {
  // No row has the focus: the first with no verdict gets it, and the focus
  // goes to the first radio of the next with none.
  let form = approvalForm([[2], [0], [0]]);
  let press = setup({ form });
  assert.equal(press("3"), true);
  assert.deepEqual(log, ["click row1.3", "focus row2.1"]);
  // That row now has the focus, so the next key sets it.
  log = [];
  assert.equal(press("1"), true);
  assert.deepEqual(log, ["click row2.1", "focus row2.1"]);
  // A pre-selected verdict counts as none.
  form = approvalForm([[1, true], [1, true]]);
  press = setup({ form });
  assert.equal(press("2"), true);
  assert.deepEqual(log, ["click row0.2", "focus row1.1"]);
});

test("on an approval a number key with no row or verdict to set, or held, does nothing", () => {
  for (const [rows, key, init] of [[[[1], [2]], "1", {}], [[[0]], "4", {}], [[[0]], "1", { repeat: true }]]) {
    const press = setup({ form: approvalForm(rows) });
    assert.equal(press(key, init), false);
    assert.deepEqual(log, []);
  }
});
