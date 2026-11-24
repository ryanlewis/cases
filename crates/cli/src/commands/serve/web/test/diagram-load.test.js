import { test } from "node:test";
import assert from "node:assert/strict";
import { install, mermaid, settle } from "./diagram-dom.js";
import { draw } from "../static/diagram.ts";

// A diagram drawn while mermaid does not load, in a page of its own: each
// test file runs in a process of its own, so mermaid is not loaded yet here.
// One test, as mermaid stays loaded once it loads: a test after it could
// not have mermaid fail to load.
const { document, observers } = install();

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

test("a diagram not drawn as mermaid did not load is tried again on the next draw or change, with one line while it waits, and drawn once it loads; one whose text then does not parse is not tried again", async () => {
  mermaid.fail = true;
  const good = fence("flowchart LR\n  A --> B");
  const bad = fence("flowchart LR\n  A -->");
  draw(document.body);
  await settle();
  assert.equal(error(good), "not drawn: mermaid did not load");
  assert.equal(error(bad), "not drawn: mermaid did not load");
  let imports = mermaid.imports;
  draw(document.body);
  await settle();
  assert.ok(mermaid.imports > imports, "a draw tries the import again");
  assert.equal(good.querySelectorAll(".diagram-error").length, 1, "one line, not one a try");
  imports = mermaid.imports;
  observers[0].callback();
  await settle();
  assert.ok(mermaid.imports > imports, "a change of look or theme tries the import again");
  assert.equal(good.querySelectorAll(".diagram-error").length, 1);
  mermaid.fail = false;
  mermaid.svg = (id, source) => {
    if (source.endsWith("-->")) throw new Error("Parse error on line 2:\nExpecting 'AMP', got 'EOF'");
    return "<svg><text>drawn</text></svg>";
  };
  observers[0].callback();
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
  observers[0].callback();
  await settle();
  assert.deepEqual(mermaid.renders.slice(renders).map((r) => r.source), ["flowchart LR\n  A --> B"], "a change draws the drawn diagram again, once, and not the one that does not parse");
  good.remove();
  bad.remove();
});
