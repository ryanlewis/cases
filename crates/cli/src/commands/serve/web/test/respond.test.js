// The answer form (static/case.ts, IN-03): what `response` makes of a
// form's fields for each kind, and what `Respond` draws for each kind and
// state, what each of its controls does to the fields, and what its submit,
// drop and park send, and the submits of a message's edit and of a
// withdraw. render-to-string runs no effect and never draws
// again, so these tests give case.ts the hooks in respond-hooks.js, mount
// Respond with them, call the handlers in the tree it gives, and draw the
// tree to HTML to see what the form shows.
//
// The hooks are case.ts's only if this file loads case.ts first, so it
// needs a process of its own, as `node --test` gives each file by default.
import { test } from "node:test";
import assert from "node:assert/strict";
import { registerHooks } from "node:module";
import { options } from "preact";
import { render } from "preact-render-to-string";

const shim = new URL("./respond-hooks.js", import.meta.url).href;
registerHooks({
  resolve(specifier, context, next) {
    if (specifier === "preact/hooks" && context.parentURL?.endsWith("/static/case.ts")) return next(shim, context);
    return next(specifier, context);
  },
});
const { mount } = await import("./hooks-stand-in.js");
const { html } = await import("../static/html.ts");
const { CaseView, response } = await import("../static/case.ts");
const { SEND } = await import("../static/keys.ts");
const { fields } = await import("./case.js");

// An open case of `kind`, with `extra` over the rest.
const make = (kind, extra = {}) => ({ id: "c1Ab2", kind, state: "open", urgency: "today", title: "t", body: "b", events: [], revision: 1, amend_seq: 0, ...extra });
const rows = [
  { id: "r1", label: "row one", link: "", script: "" },
  { id: "r2", label: "row two", link: "", script: "", by: "human" },
];
const decision = make("decision", { options: ["one", "two"], recommend: 2 });
const approval = make("approval", { rows });
const signoff = make("signoff");
const stuck = make("stuck");
const question = make("question");
const fyi = make("fyi");
const briefing = make("briefing", { left_out: ["the docs", "a bench", "the TUI"] });

// --- response ---------------------------------------------------------------

// Each row: the case, the fields set, preselect, and what response gives.
const said = (c, f, preselect, want) => ({ c, f, preselect, want });
const problem = (text) => ({ problem: text });
const answer = (a) => ({ answer: a });

const decisions = [
  said(decision, {}, false, problem("choose an option")),
  said(decision, { note: "  why  " }, false, problem("choose an option")),
  said(decision, { choice: "2" }, false, answer({ choice: 2 })),
  said(decision, { choice: "1", note: " a note\n" }, false, answer({ note: "a note", choice: 1 })),
  // The number is not checked against the options: the store does that.
  said(decision, { choice: "9" }, false, answer({ choice: 9 })),
  said(decision, { choice: "other" }, false, problem("other needs a note: say what you want instead")),
  said(decision, { choice: "other", note: " \t\n" }, false, problem("other needs a note: say what you want instead")),
  said(decision, { choice: "other", note: " a print " }, false, answer({ note: "a print", other: true })),
  // A blank note is left out, not sent empty.
  said(decision, { choice: "1", note: "   " }, false, answer({ choice: 1 })),
];

const approvals = [
  said(approval, {}, false, problem(`choose approve, hold or don't run for "row one"`)),
  said(approval, { verdicts: { r1: "hold" } }, false, problem('choose done, hold or won\'t do for "row two"')),
  // Pre-selected, a row the agent runs is approved; the human's row is not.
  said(approval, {}, true, problem('choose done, hold or won\'t do for "row two"')),
  said(approval, { verdicts: { r2: "approve" } }, true, answer({ rows: [{ id: "r1", verdict: "approve" }, { id: "r2", verdict: "approve" }] })),
  said(approval, { verdicts: { r1: "reject", r2: "hold" } }, true, answer({ rows: [{ id: "r1", verdict: "reject" }, { id: "r2", verdict: "hold" }] })),
  said(approval, { verdicts: { r1: "approve", r2: "reject" }, rowNotes: { r1: "  look  ", r2: "  " }, note: " all " }, false,
    answer({ note: "all", rows: [{ id: "r1", verdict: "approve", note: "look" }, { id: "r2", verdict: "reject" }] })),
  // A row note on a row the case has not is not sent.
  said(approval, { verdicts: { r1: "approve", r2: "approve" }, rowNotes: { r9: "x" } }, false, answer({ rows: [{ id: "r1", verdict: "approve" }, { id: "r2", verdict: "approve" }] })),
  // A row id that names a property of every object has no verdict until one is chosen.
  said(make("approval", { rows: [{ id: "constructor", label: "c" }] }), {}, false, problem(`choose approve, hold or don't run for "c"`)),
  said(make("approval", { rows: [{ id: "__proto__", label: "p" }] }), { rowNotes: {} }, true, answer({ rows: [{ id: "__proto__", verdict: "approve" }] })),
  // No rows: an empty list, which the store refuses (it never opens one).
  said(make("approval"), {}, false, answer({ rows: [] })),
];

const signoffs = [
  said(signoff, {}, false, problem("choose accept or request changes")),
  said(signoff, { signoff: "accept" }, false, answer({ signoff: "accept" })),
  said(signoff, { signoff: "accept", note: " fine " }, false, answer({ note: "fine", signoff: "accept" })),
  said(signoff, { signoff: "changes" }, false, problem("requesting changes needs a comment: say what to change")),
  said(signoff, { signoff: "changes", note: "  " }, false, problem("requesting changes needs a comment: say what to change")),
  said(signoff, { signoff: "changes", note: " rename it " }, false, answer({ note: "rename it", signoff: "changes" })),
];

const stucks = [
  said(stuck, {}, false, problem("choose guidance or drop, or park it")),
  said(stuck, { text: "   " }, false, problem("choose guidance or drop, or park it")),
  said(stuck, { stuck: "text" }, false, problem("write the guidance, or choose drop")),
  said(stuck, { stuck: "text", text: " \n " }, false, problem("write the guidance, or choose drop")),
  said(stuck, { stuck: "text", text: " try this " }, false, answer({ text: "try this" })),
  // Guidance typed without its button chosen is guidance.
  said(stuck, { text: " try this " }, false, answer({ text: "try this" })),
  said(stuck, { stuck: "drop" }, false, answer({ drop: true })),
  // Drop chosen sends drop, whatever guidance is typed.
  said(stuck, { stuck: "drop", text: "ignored", note: " why " }, false, answer({ note: "why", drop: true })),
];

const questions = [
  said(question, {}, false, problem("write the reply")),
  said(question, { text: " \t " }, false, problem("write the reply")),
  said(question, { text: " yes \n", note: " n " }, false, answer({ note: "n", text: "yes" })),
];

const fyis = [
  said(fyi, {}, false, answer({ ack: true })),
  said(fyi, { note: " seen " }, false, answer({ note: "seen", ack: true })),
];

const briefings = [
  said(briefing, {}, false, problem("choose accept or send back")),
  said(briefing, { file: [1], note: "x" }, false, problem("choose accept or send back")),
  said(briefing, { briefing: "accept" }, false, answer({ briefing: "accept" })),
  said(briefing, { briefing: "accept", file: [3, 1] }, false, answer({ briefing: "accept", file: [1, 3] })),
  // A tick past the list files nothing.
  said(briefing, { briefing: "accept", file: [4, 2] }, false, answer({ briefing: "accept", file: [2] })),
  said(make("briefing"), { briefing: "accept", file: [1] }, false, answer({ briefing: "accept" })),
  // Pinned as it is: a tick below the list is kept, and the store refuses
  // it. The form never makes one: its ticks count from 1.
  said(briefing, { briefing: "accept", file: [0] }, false, answer({ briefing: "accept", file: [0] })),
  said(briefing, { briefing: "send_back" }, false, problem("sending back needs a reason: say what to fix")),
  said(briefing, { briefing: "send_back", note: " \n" }, false, problem("sending back needs a reason: say what to fix")),
  said(briefing, { briefing: "send_back", note: " split it ", file: [2] }, false, answer({ note: "split it", briefing: "send_back", file: [2] })),
];

// A kind the form has no case for sends the note alone; a message, the
// only one, is never open, so the form is never drawn for it.
const others = [
  said(make("message"), {}, false, answer({})),
  said(make("message"), { note: " n " }, false, answer({ note: "n" })),
];

for (const [kind, table] of Object.entries({ decisions, approvals, signoffs, stucks, questions, fyis, briefings, others })) {
  test(`response gives each of the ${kind}' problems with its text, or the exact answer`, () => {
    for (const { c, f, preselect, want } of table) {
      assert.deepEqual(response(c, fields(f), preselect), want, `${JSON.stringify(f)} preselect=${preselect}`);
    }
  });
}

test("response leaves the fields it is given as they are", () => {
  const f = fields({ choice: "1", verdicts: { r1: "approve" }, file: [3, 1], note: " n " });
  const was = structuredClone(f);
  response(decision, f, true);
  response(approval, f, true);
  response({ ...briefing }, { ...f, briefing: "accept" }, false);
  assert.deepEqual(f, was);
});

// --- Respond ----------------------------------------------------------------

// Every vnode of `tree`, in order.
function* nodes(tree) {
  if (Array.isArray(tree)) {
    for (const child of tree) yield* nodes(child);
    return;
  }
  if (!tree || typeof tree !== "object") return;
  yield tree;
  yield* nodes(tree.props?.children);
}

// The vnode of component `name` CaseView draws for case `c`.
function respondOf(c, name = "Respond") {
  const was = options.vnode;
  let found;
  options.vnode = (v) => {
    if (typeof v.type === "function" && v.type.name === name) found = v;
    if (was) was(v);
  };
  try {
    render(html`<${CaseView} c=${c} now=${0} onSent=${() => {}} onCase=${() => {}} />`);
  } finally {
    options.vnode = was;
  }
  assert.ok(found, `CaseView drew no ${name} for ${c.kind} ${c.state}`);
  return found;
}

// The form for case `c`, mounted, with the options pre-selecting approve
// when `preselect`, and `last` the answer a follow-up reopened. What it
// sends and refuses is logged.
function form(c, { preselect = false, last, component } = {}) {
  const v = respondOf(c, component);
  const sends = [];
  const refusals = [];
  const props = { ...v.props, c, last, send: async (what, body) => sends.push([what, body]), refuse: (p) => refusals.push(p) };
  const was = globalThis.window;
  globalThis.window = { casesPrefs: { read: (key) => (key === "approve" && preselect ? "on" : "off") } };
  let m;
  try {
    m = mount(v.type, props);
  } finally {
    globalThis.window = was;
  }
  const find = (pick, what) => {
    const found = [...nodes(m.tree)].find((n) => n.props && pick(n));
    assert.ok(found, `the form has no ${what}`);
    return found.props;
  };
  const self = {
    sends,
    refusals,
    props,
    get html() {
      return render(m.tree);
    },
    // Picks the radio `name` `value`, as a click does.
    choose(name, value) {
      find((n) => n.props.name === name && n.props.value === value && n.props.onPick, `${name}=${value}`).onPick(value);
      m.draw();
      return self;
    },
    // Types `text` into the field `name`, all of it, as an input does.
    type(name, text) {
      find((n) => n.props.name === name && n.props.onInput, `field ${name}`).onInput(text);
      m.draw();
      return self;
    },
    // Ticks or unticks the left-out item `n`.
    tick(n, checked = true) {
      find((n2) => n2.type === "input" && n2.props.name === "file" && n2.props.value === String(n), `tick ${n}`).onChange({ target: { checked } });
      m.draw();
      return self;
    },
    // Submits the form, and gives whether the browser's own submit was stopped.
    submit() {
      let stopped = false;
      find((n) => n.type === "form", "form").onSubmit({ preventDefault: () => (stopped = true) });
      m.draw();
      return stopped;
    },
    click(id) {
      find((n) => n.type === "button" && n.props.id === id, `button #${id}`).onClick();
      m.draw();
      return self;
    },
    // Draws the form again for the case `next`, as an amend that lands does.
    redraw(next) {
      props.c = next;
      m.dirty = true;
      m.draw();
      return self;
    },
  };
  return self;
}

const hint = (keys) => `<span class="key-hint" aria-hidden="true">${keys.map((k) => `<kbd>${k}</kbd>`).join("")}</span>`;
const choice = (name, value, n, text, { required = true, checked = false } = {}) =>
  `<label class="choice"><input type="radio" name="${name}" value="${value}"${required ? " required" : ""}${checked ? " checked" : ""} aria-keyshortcuts="${n}"/> ${text}${hint([String(n)])}</label>`;
const note = (label, required = false) => `<label>${label} ${hint(["n"])} <textarea name="note" rows="3"${required ? " required" : ""} aria-keyshortcuts="n"></textarea></label>`;
const actions = (word, other) =>
  `<div class="actions"><button type="submit" class="primary" id="respond-send" aria-keyshortcuts="Meta+Enter Control+Enter">${word} ${hint(SEND)}</button>${other}</div>`;
const drop = '<button type="button" id="respond-drop">drop</button>';
const park = '<button type="button" id="respond-park">park</button>';
const wrap = (inner) => `<form class="respond" data-write="answer">${inner}</form>`;

test("a decision's form: its options numbered, the recommended and last pick marked, other, the note, send and drop", () => {
  const recommended = ' <span class="badge recommended">recommended</span>';
  assert.equal(form(decision).html, wrap(
    `<fieldset><legend>choose one</legend>${choice("choice", 1, 1, "one")}${choice("choice", 2, 2, `<span>two${recommended}</span>`)}${choice("choice", "other", 3, "other, see note")}</fieldset>${note("note to the agent")}${actions("send", drop)}`,
  ));
  const page = form(decision, { last: { other: true } }).html;
  assert.ok(page.includes(choice("choice", "other", 3, '<span>other, see note <span class="badge last-pick">your last pick</span></span>')), page);
  const again = form({ ...decision, recommend: undefined }, { last: { choice: 1 } }).html;
  assert.ok(again.includes(choice("choice", 1, 1, '<span>one <span class="badge last-pick">your last pick</span></span>')) && !again.includes("recommended"), again);
  // A decision with no options offers other alone.
  assert.ok(form(make("decision")).html.includes(`<legend>choose one</legend>${choice("choice", "other", 1, "other, see note")}</fieldset>`));
});

test("a decision's note is required while other is chosen, and the choice is checked", () => {
  const f = form(decision).choose("choice", "other");
  assert.ok(f.html.includes(choice("choice", "other", 3, "other, see note", { checked: true })) && f.html.includes(note("note to the agent", true)), f.html);
  f.choose("choice", "1");
  assert.ok(f.html.includes(choice("choice", 1, 1, "one", { checked: true })) && f.html.includes(note("note to the agent")), f.html);
});

test("a signoff's form: accept, request changes, the comment, required while changes is chosen", () => {
  const fieldset = (checked) => `<fieldset><legend>sign off</legend>${choice("signoff", "accept", 1, "accept", { checked: checked === "accept" })}${choice("signoff", "changes", 2, "request changes (comment required)", { checked: checked === "changes" })}</fieldset>`;
  const f = form(signoff);
  assert.equal(f.html, wrap(`${fieldset()}${note("comment")}${actions("send", drop)}`));
  assert.equal(f.choose("signoff", "changes").html, wrap(`${fieldset("changes")}${note("comment", true)}${actions("send", drop)}`));
  assert.equal(f.choose("signoff", "accept").html, wrap(`${fieldset("accept")}${note("comment")}${actions("send", drop)}`));
});

test("a briefing's form: accept, send back, a tick box per left-out item, and the reason, named and required while sending back", () => {
  const tick = (n, item, checked = false) => `<label class="choice"><input type="checkbox" name="file" value="${n}"${checked ? " checked" : ""}/> ${item}</label>`;
  const fieldset = (checked) => `<fieldset><legend>judge the work</legend>${choice("briefing", "accept", 1, "accept", { checked: checked === "accept" })}${choice("briefing", "send_back", 2, "send back (reason required)", { checked: checked === "send_back" })}</fieldset>`;
  const files = (...on) => `<fieldset class="file"><legend>file what was left out</legend>${briefing.left_out.map((item, i) => tick(i + 1, item, on.includes(i + 1))).join("")}</fieldset>`;
  const f = form(briefing);
  assert.equal(f.html, wrap(`${fieldset()}${files()}${note("note to the agent")}${actions("send", drop)}`));
  f.choose("briefing", "send_back").tick(2);
  assert.equal(f.html, wrap(`${fieldset("send_back")}${files(2)}${note("reason: what to fix", true)}${actions("send", drop)}`));
  f.choose("briefing", "accept");
  assert.equal(f.html, wrap(`${fieldset("accept")}${files(2)}${note("note to the agent")}${actions("send", drop)}`));
  // No left-out items, no tick boxes.
  for (const left_out of [undefined, []]) assert.equal(form(make("briefing", { left_out })).html, wrap(`${fieldset()}${note("note to the agent")}${actions("send", drop)}`));
});

test("a stuck case's form: guidance and its text, drop it, the note, send and park", () => {
  const f = form(stuck);
  const fieldset = (checked) => `<fieldset><legend>unblock</legend>${choice("stuck", "text", 1, "give guidance", { required: false, checked: checked === "text" })}<label>guidance <textarea name="text" rows="4"></textarea></label>${choice("stuck", "drop", 2, "drop it", { required: false, checked: checked === "drop" })}</fieldset>`;
  assert.equal(f.html, wrap(`${fieldset()}${note("note to the agent")}${actions("send", park)}`));
  assert.equal(f.choose("stuck", "drop").html, wrap(`${fieldset("drop")}${note("note to the agent")}${actions("send", park)}`));
});

test("a question's form is its reply, required; an fyi's is the note and acknowledge", () => {
  assert.equal(form(question).html, wrap(`<label>reply <textarea name="text" rows="4" required></textarea></label>${note("note to the agent")}${actions("send", drop)}`));
  assert.equal(form(fyi).html, wrap(`${note("note to the agent")}${actions("acknowledge", drop)}`));
});

test("an approval's form: per row its title, note, verdicts numbered and its note field; the human's row tagged with its own words", () => {
  const c = make("approval", { rows: [{ ...rows[0], note: "the *row*", link: "https://example.com/a" }, rows[1]] });
  const page = form(c).html;
  assert.ok(page.startsWith('<form class="respond" data-write="answer"><fieldset class="row"><legend class="row-title">row one</legend><p class="context row-note">the <em>row</em></p>'), page);
  assert.ok(page.includes('<p><a href="https://example.com/a"'), page);
  const verdicts = (id, words, checked) => `<div class="verdicts">${words.map(([value, word, hintText], i) => `<label class="choice"><input type="radio" name="verdict.${id}" value="${value}" required${checked === value ? " checked" : ""} aria-keyshortcuts="${i + 1}"/> ${word} <span class="hint">${hintText}</span>${hint([String(i + 1)])}</label>`).join("")}</div><label>note on this row <textarea name="note.${id}" rows="1"></textarea></label></fieldset>`;
  const agentWords = [["approve", "approve", "run it"], ["hold", "hold", "not now; ask again later"], ["reject", "don't run", "never run this row"]];
  const humanWords = [["approve", "done", "you did it"], ["hold", "hold", "not yet; ask again later"], ["reject", "won't do", "you won't do this step"]];
  assert.ok(page.includes(verdicts("r1", agentWords)), page);
  assert.ok(page.includes('<fieldset class="row" data-by="human"><legend class="row-title">row two <span class="badge by-human" title="a step you carry out">you</span></legend>'), page);
  assert.ok(page.includes(verdicts("r2", humanWords)), page);
  assert.ok(page.endsWith(`${note("note to the agent")}${actions("send", drop)}</form>`), page);
  // Pre-selected, the agent's row starts approved and marked as preset; a
  // verdict chosen is no longer a preset; the human's row is never preset.
  const f = form(c, { preselect: true });
  assert.ok(f.html.includes('<fieldset class="row" data-preset="true"><legend class="row-title">row one</legend>') && f.html.includes(verdicts("r1", agentWords, "approve")), f.html);
  assert.ok(f.html.includes(verdicts("r2", humanWords)), f.html);
  f.choose("verdict.r1", "hold").choose("verdict.r2", "approve");
  assert.ok(f.html.includes('<fieldset class="row"><legend class="row-title">row one</legend>') && f.html.includes(verdicts("r1", agentWords, "hold")), f.html);
  assert.ok(f.html.includes(verdicts("r2", humanWords, "approve")), f.html);
  // The row note typed is shown in its field.
  assert.ok(f.type("note.r1", "careful").html.includes('<textarea name="note.r1" rows="1">careful</textarea>'), f.html);
});

test("the options are read once: a pre-select turned on while the form is open leaves it be", () => {
  const f = form(approval);
  const was = globalThis.window;
  globalThis.window = { casesPrefs: { read: () => "on" } };
  try {
    f.redraw({ ...approval });
    f.submit();
  } finally {
    globalThis.window = was;
  }
  assert.deepEqual(f.refusals, [`choose approve, hold or don't run for "row one"`]);
});

test("submit sends the answer the fields give, and stops the browser's own submit", () => {
  const cases = [
    [decision, (f) => f.choose("choice", "2").type("note", " because ")],
    [decision, (f) => f.choose("choice", "other").type("note", "a print")],
    [approval, (f) => f.choose("verdict.r1", "reject").choose("verdict.r2", "approve").type("note.r2", " done it ").type("note", "n")],
    [signoff, (f) => f.choose("signoff", "changes").type("note", "rename it")],
    [stuck, (f) => f.type("text", " try this ")],
    [stuck, (f) => f.choose("stuck", "drop")],
    [question, (f) => f.type("text", "yes")],
    [fyi, (f) => f],
    [briefing, (f) => f.choose("briefing", "accept").tick(3).tick(1).tick(2).tick(2, false).tick(1)],
  ];
  const want = [
    { note: "because", choice: 2 },
    { note: "a print", other: true },
    { note: "n", rows: [{ id: "r1", verdict: "reject" }, { id: "r2", verdict: "approve", note: "done it" }] },
    { note: "rename it", signoff: "changes" },
    { text: "try this" },
    { drop: true },
    { text: "yes" },
    { ack: true },
    { briefing: "accept", file: [1, 3] },
  ];
  cases.forEach(([c, fill], i) => {
    const f = fill(form(c));
    assert.equal(f.submit(), true);
    assert.deepEqual(f.sends, [["answer", { answer: want[i] }]], `${c.kind} ${i}`);
    assert.deepEqual(f.refusals, []);
  });
});

test("approve pre-selected, submit sends the agent's rows approved", () => {
  const f = form(approval, { preselect: true }).choose("verdict.r2", "hold");
  f.submit();
  assert.deepEqual(f.sends, [["answer", { answer: { rows: [{ id: "r1", verdict: "approve" }, { id: "r2", verdict: "hold" }] } }]]);
});

test("submit with a problem shows it and sends nothing, and the fields are kept", () => {
  for (const [c, fill, problem] of [
    [decision, (f) => f, "choose an option"],
    [decision, (f) => f.choose("choice", "other").type("note", "  "), "other needs a note: say what you want instead"],
    [signoff, (f) => f.choose("signoff", "changes"), "requesting changes needs a comment: say what to change"],
    [briefing, (f) => f.choose("briefing", "send_back"), "sending back needs a reason: say what to fix"],
    [approval, (f) => f.choose("verdict.r1", "approve"), 'choose done, hold or won\'t do for "row two"'],
    [question, (f) => f, "write the reply"],
  ]) {
    const f = fill(form(c));
    const before = f.html;
    assert.equal(f.submit(), true);
    assert.deepEqual(f.refusals, [problem], c.kind);
    assert.deepEqual(f.sends, [], c.kind);
    assert.equal(f.html, before, c.kind);
  }
});

test("drop sends drop with the note, trimmed, whatever else is chosen; park sends the note alone", () => {
  for (const c of [decision, approval, signoff, question, fyi, briefing]) {
    const f = form(c);
    f.click("respond-drop");
    f.type("note", " not now ").click("respond-drop");
    f.type("note", "   ").click("respond-drop");
    assert.deepEqual(f.sends, [["answer", { answer: { drop: true } }], ["answer", { answer: { drop: true, note: "not now" } }], ["answer", { answer: { drop: true } }]], c.kind);
  }
  const f = form(stuck).type("text", "guidance");
  f.click("respond-park");
  f.type("note", " later ").click("respond-park");
  f.type("note", " ").click("respond-park");
  assert.deepEqual(f.sends, [["park", {}], ["park", { note: "later" }], ["park", {}]]);
});

test("a tick is held once, and an untick takes it back", () => {
  const f = form(briefing).choose("briefing", "accept").tick(2).tick(2).tick(3).tick(3, false).tick(1, false);
  f.submit();
  assert.deepEqual(f.sends, [["answer", { answer: { briefing: "accept", file: [2] } }]]);
});

test("an amend that replaces a briefing's left-out items clears the ticks; a redraw with the same items keeps them, and the rest of the form", () => {
  const f = form(briefing).choose("briefing", "accept").tick(1).tick(3).type("note", "kept");
  f.redraw({ ...briefing, left_out: [...briefing.left_out], revision: 2 });
  f.submit();
  assert.deepEqual(f.sends, [["answer", { answer: { note: "kept", briefing: "accept", file: [1, 3] } }]]);
  f.redraw({ ...briefing, left_out: ["the docs", "the TUI", "a bench"], revision: 3 });
  assert.ok(!/type="checkbox"[^>]* checked/.test(f.html), f.html);
  f.submit();
  assert.deepEqual(f.sends[1], ["answer", { answer: { note: "kept", briefing: "accept" } }]);
  // A case with no items that gains some clears nothing it had not.
  const g = form(make("briefing")).choose("briefing", "accept").redraw({ ...briefing }).tick(2);
  g.submit();
  assert.deepEqual(g.sends, [["answer", { answer: { briefing: "accept", file: [2] } }]]);
});

// The other two forms with a submit of their own, as the same harness reaches them.

test("a sent message's edit sends the body, trimmed, and refuses a blank one or one unchanged", () => {
  const sent = make("message", { state: "sent", worker: "w1", body: "Rebase onto main." });
  const f = form(sent, { component: "Edit" });
  f.submit();
  f.type("body", "  Rebase onto main.  ").submit();
  f.type("body", " \n ").submit();
  f.type("body", " Rebase onto main, then push. ").submit();
  assert.deepEqual(f.refusals, ["the message is as it was, so there is nothing to save", "the message is as it was, so there is nothing to save", "write the message, or withdraw it"]);
  assert.deepEqual(f.sends, [["amend", { body: "Rebase onto main, then push." }]]);
});

test("a withdraw sends the reason, trimmed, or none", () => {
  const f = form(make("decision", { state: "answered", options: ["one"] }), { component: "Withdraw" });
  assert.equal(f.submit(), true);
  f.type("reason", "  its session died ").submit();
  f.type("reason", "  ").submit();
  assert.deepEqual(f.sends, [["withdraw", {}], ["withdraw", { reason: "its session died" }], ["withdraw", {}]]);
});
