// How often the case view turns its markdown and code into HTML (B1): once
// per text, not on every keystroke in the form or every redraw of the same
// case. case.ts gets counting stand-ins for markdown.js and code.js
// (parse-spies.js), and the hooks in respond-hooks.js, so that the answer
// form can be mounted and typed into, as respond.test.js does. A keystroke
// redraws Respond and then every component in the tree it gives, as preact
// does: the test draws that tree to HTML after each one.
//
// The stand-ins are case.ts's only if this file loads case.ts first, so it
// needs a process of its own, as `node --test` gives each file by default.
import { test } from "node:test";
import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import { options } from "preact";
import { render } from "preact-render-to-string";

const here = (file) => new URL(file, import.meta.url).href;
registerHooks({
  resolve(specifier, context, next) {
    if (context.parentURL?.endsWith("/static/case.ts")) {
      if (specifier === "preact/hooks") return next(here("./respond-hooks.js"), context);
      if (specifier === "./markdown.js" || specifier === "./code.js") return next(here("./parse-spies.js"), context);
    }
    return next(specifier, context);
  },
});
const { mount } = await import("./hooks-stand-in.js");
const { html } = await import("../static/html.ts");
const { CaseView } = await import("../static/case.ts");
const { parses } = await import("./parse-spies.js");

// The counts since the last call, and zeroes them.
function taken() {
  const was = { ...parses };
  for (const key of Object.keys(parses)) parses[key] = 0;
  return was;
}

const body = "## The plan\n\nRun the **migration**:\n\n```sh\ncargo run -- migrate\n```\n";
const row = (n) => ({ id: `r${n}`, label: `row ${n}`, link: "", script: `echo ${n}`, note: `see *step ${n}*` });
const approval = { id: "c1Ab2", kind: "approval", state: "open", urgency: "today", title: "t", body, rows: [row(1), row(2), row(3)], events: [], revision: 1, amend_seq: 0 };
const briefing = { ...approval, kind: "briefing", rows: undefined, left_out: ["the docs", "a bench"] };

// The case view of `c`, drawn as serve's stream redraws it.
const view = (c) => render(html`<${CaseView} c=${c} now=${0} onSent=${() => {}} onCase=${() => {}} />`);

// The answer form of `c`, mounted, with `type` and `tick` that redraw it
// and every component in it.
function form(c) {
  const was = options.vnode;
  let found;
  options.vnode = (v) => {
    if (typeof v.type === "function" && v.type.name === "Respond") found = v;
    if (was) was(v);
  };
  try {
    view(c);
  } finally {
    options.vnode = was;
  }
  const m = mount(found.type, { ...found.props, send: async () => {}, refuse: () => {} });
  render(m.tree);
  // The props of the first vnode in the tree that `pick` picks.
  const find = (pick, tree) => {
    if (Array.isArray(tree)) {
      for (const child of tree) {
        const hit = find(pick, child);
        if (hit) return hit;
      }
      return undefined;
    }
    if (!tree || typeof tree !== "object") return undefined;
    if (tree.props && pick(tree)) return tree.props;
    return find(pick, tree.props?.children);
  };
  const redraw = () => {
    m.draw();
    render(m.tree);
  };
  return {
    type(name, text) {
      find((v) => v.props.name === name && v.props.onInput, m.tree).onInput(text);
      redraw();
    },
    tick(n) {
      find((v) => v.type === "input" && v.props.name === "file" && v.props.value === String(n), m.tree).onChange({ target: { checked: true } });
      redraw();
    },
  };
}

const none = { markdown: 0, inlineMarkdown: 0, codeBlock: 0 };

test("the first draw of a case parses its body, each row's note and each row's script once", () => {
  taken();
  view(approval);
  assert.deepEqual(taken(), { markdown: 1, inlineMarkdown: 3, codeBlock: 3 });
});

test("a keystroke in the note, in a row's note or a tick parses nothing again", () => {
  const f = form(approval);
  taken();
  f.type("note", "a");
  assert.deepEqual(taken(), none, "the note");
  f.type("note", "ab");
  assert.deepEqual(taken(), none, "the note again");
  f.type("note.r2", "x");
  assert.deepEqual(taken(), none, "a row's note");
  const g = form(briefing);
  taken();
  g.tick(2);
  assert.deepEqual(taken(), none, "a tick");
});

test("a redraw of the same case parses nothing again; an amend that changes the body parses the new body, once", () => {
  view(approval);
  taken();
  view({ ...approval, revision: 2, rows: approval.rows.map((r) => ({ ...r })) });
  assert.deepEqual(taken(), none, "a new revision, the same texts");
  const amended = { ...approval, revision: 3, body: `${body}\nAnd then *check* it.\n`, rows: [approval.rows[0], { ...row(2), note: "a new note", script: "echo two" }, approval.rows[2]] };
  const page = view(amended);
  assert.deepEqual(taken(), { markdown: 1, inlineMarkdown: 1, codeBlock: 1 }, "the amend");
  // What is drawn is the new text, never the old.
  assert.ok(page.includes("<em>check</em>") && page.includes("a new note") && page.includes("echo two"), page);
  assert.ok(!page.includes("step 2") && !page.includes("echo 2<"), page);
  view(amended);
  assert.deepEqual(taken(), none, "the amended case again");
});

test("a script is drawn again when the wrap option changes, and only then", () => {
  view(approval);
  taken();
  const was = globalThis.window;
  globalThis.window = { casesPrefs: { read: (key) => (key === "wrap" ? "wrap" : "") } };
  try {
    const page = view(approval);
    assert.deepEqual(taken(), { ...none, codeBlock: 3 });
    assert.ok(page.includes('aria-pressed="true"'), page);
    view(approval);
    assert.deepEqual(taken(), none);
  } finally {
    globalThis.window = was;
  }
});
