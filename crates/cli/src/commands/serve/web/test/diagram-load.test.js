import { test } from "node:test";
import assert from "node:assert/strict";
import { install, mermaid, settle } from "./diagram-dom.js";
import { draw } from "../static/diagram.ts";

// A diagram drawn while mermaid does not load, in a page of its own: each
// test file runs in a process of its own, so mermaid is not loaded yet here.
// These tests run in order: the first has mermaid load at last.
const { document } = install();

function fence(source) {
  const el = document.createElement("div");
  el.className = "diagram";
  const code = document.createElement("code");
  code.textContent = source;
  el.append(code);
  document.body.append(el);
  return el;
}

const error = (el) => el.querySelector(".diagram-error")?.textContent;
const drawnSvg = (el) => el.shadowRoot.children[0];

test("a diagram not drawn as mermaid did not load is drawn once it loads, with one line while it waits; one whose text then does not parse is not tried again", async () => {
  mermaid.fail = true;
  const good = fence("flowchart LR\n  A --> B");
  const bad = fence("flowchart LR\n  A -->");
  draw(document.body);
  await settle();
  assert.equal(error(good), "not drawn: mermaid did not load");
  assert.equal(error(bad), "not drawn: mermaid did not load");
  draw(document.body);
  await settle();
  assert.equal(mermaid.imports, 4, "each diagram's turn, on each draw, tries the import again");
  assert.equal(good.querySelectorAll(".diagram-error").length, 1, "one line, not one a try");
  mermaid.fail = false;
  mermaid.svg = (id, source) => {
    if (source.endsWith("-->")) throw new Error("Parse error on line 2:\nExpecting 'AMP', got 'EOF'");
    return "<svg><text>drawn</text></svg>";
  };
  draw(document.body);
  await settle();
  assert.equal(drawnSvg(good).textContent, "drawn");
  assert.equal(error(good), undefined, "the line goes once it is drawn");
  assert.ok(good.querySelector("code"), "the code block stays in the light tree");
  assert.equal(error(bad), "not drawn: Parse error on line 2: Expecting 'AMP', got 'EOF'");
  assert.equal(bad.querySelectorAll(".diagram-error").length, 1);
  const renders = mermaid.renders.length;
  draw(document.body);
  await settle();
  assert.equal(mermaid.renders.length, renders, "a parse failure is for good, and a drawn diagram is not drawn again");
  good.remove();
  bad.remove();
});
