import { test } from "node:test";
import assert from "node:assert/strict";
import { render } from "preact-render-to-string";
import { html } from "../static/html.ts";
import { Page } from "../static/page.ts";
import { decision, steps } from "./case.js";

const now = Date.parse("2026-09-25T13:45:00Z");
const home = (show = "inbox") => ({ page: "list", show });
const draw = (data) => render(html`<${Page} data=${data} now=${now} sent=${() => {}} refused=${() => {}} />`);

// A case the human answered at 13:20, with an agent now.
const answered = { ...decision("answered", steps.open, steps.answer), answer: { answered_at: steps.answer.at } };

// The list column and the case pane of `page`, which must have both.
function columns(page) {
  const list = page.indexOf('<aside class="list"');
  const work = page.indexOf('<div class="work">');
  assert.ok(list >= 0 && work > list, page);
  return { list: page.slice(list, work), work: page.slice(work) };
}

test("inbox zero is the split: the list with its filters beside the panel", () => {
  const page = draw({ route: home(), show: "inbox", inbox: [], done: [answered], shown: null });
  assert.ok(page.startsWith('<div class="split home zero">'), page);
  assert.ok(!page.includes('class="zero"'), page);
  const { list, work } = columns(page);
  assert.ok(list.includes('aria-label="inbox"'), list);
  assert.ok(list.includes('<nav class="chips filters" aria-label="filter">'), list);
  assert.ok(list.includes('inbox <span class="count">0</span>'), list);
  assert.ok(list.includes('in flight <span class="count">1</span>'), list);
  assert.ok(list.includes('<p class="empty">nothing is waiting on you.</p>'), list);
  assert.ok(work.startsWith('<div class="work"><div class="zero-panel">'), work);
  assert.ok(work.includes("<h1>nothing needs you.</h1>"), work);
  assert.ok(work.includes('<h2 class="section" id="in-flight-head">still with agents</h2>'), work);
  assert.ok(!page.includes('class="props'), page);
});

test("the first case to arrive takes the panel's place in the same split", () => {
  const c = decision("open", steps.open);
  const page = draw({ route: home(), show: "inbox", inbox: [c], done: [], shown: { id: c.id, case: c } });
  assert.ok(page.startsWith('<div class="split home">'), page);
  const { list, work } = columns(page);
  assert.ok(list.includes('class="card'), list);
  assert.ok(!list.includes("nothing is waiting on you."), list);
  assert.ok(work.includes('<header class="case-head">'), work);
  assert.ok(!page.includes("zero-panel"), page);
});

test("another empty filter says so in the list and leaves the pane empty", () => {
  for (const [show, line] of [
    ["inflight", "nothing is with an agent."],
    ["done", "nothing is done yet."],
    ["all", "there are no cases yet."],
  ]) {
    const page = draw({ route: home(show), show, inbox: [], done: [], shown: null });
    assert.ok(page.startsWith('<div class="split home">'), page);
    const { list, work } = columns(page);
    assert.ok(list.includes(`<p class="empty">${line}</p>`), list);
    assert.equal(work, '<div class="work"></div><div class="waiting-live float" aria-live="polite"></div></div>');
  }
});

test("a case's page without its case is not inbox zero", () => {
  const page = draw({ route: { page: "case", id: "zZ9", show: "inbox" }, show: "inbox", inbox: [], done: [], shown: { id: "zZ9", missing: true } });
  assert.ok(page.startsWith('<div class="split">'), page);
  assert.ok(page.includes('<p class="empty">no case zZ9 in this store.</p>'), page);
  assert.ok(!page.includes("zero-panel"), page);
});
