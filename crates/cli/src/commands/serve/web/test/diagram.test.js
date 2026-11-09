import { test } from "node:test";
import assert from "node:assert/strict";
import { install, mermaid, settle, SVG_NS, XLINK_NS } from "./diagram-dom.js";
import { config, draw, why } from "../static/diagram.ts";

test("a diagram is drawn in the page's colours and face, at mermaid's strict level", () => {
  const vars = { "--bg": "#0d0f14", "--bg-1": "#151821", "--bg-2": "#1a2a26", "--fg": "#eceef4", "--fg-2": "#9aa1b3", "--rule-strong": "#5d6579", "--sun-bg": "#151821", "--sun-fg": "#eceef4", "--sun-edge": "#5d6579", "--prose": ' "JetBrains Mono", monospace' };
  const c = config({ getPropertyValue: (name) => vars[name] ?? "", colorScheme: "dark", fontSize: "15px" }, "rgb(21, 22, 26)");
  assert.equal(c.securityLevel, "strict");
  assert.equal(c.htmlLabels, false);
  assert.equal(c.flowchart.htmlLabels, false);
  assert.equal(c.layout, "dagre");
  assert.equal(c.theme, "base");
  assert.equal(c.darkMode, true);
  assert.equal(c.fontFamily, '"JetBrains Mono", monospace');
  assert.equal(c.themeVariables.background, "rgb(21, 22, 26)");
  assert.equal(c.themeVariables.edgeLabelBackground, "rgb(21, 22, 26)");
  assert.equal(c.themeVariables.primaryColor, "#151821");
  assert.equal(c.themeVariables.primaryTextColor, "#eceef4");
  assert.equal(c.themeVariables.lineColor, "#9aa1b3");
  assert.equal(c.themeVariables.fontSize, "15px");
  // A diagram's own directives may not turn HTML labels back on, loosen
  // the sanitising, name another layout, or change the theme.
  for (const key of ["securityLevel", "htmlLabels", "flowchart", "class", "state", "layout", "theme", "themeVariables", "themeCSS", "dompurifyConfig"]) {
    assert.ok(c.secure.includes(key), key);
  }
  assert.equal(config({ getPropertyValue: () => "", colorScheme: "light" }, "#fff").darkMode, false);
});

test("why a diagram was not drawn is one short line", () => {
  assert.equal(why(new TypeError("Failed to fetch dynamically imported module: http://x/static/vendor/mermaid/chunks/mermaid.esm.min/ganttDiagram-QACGSKG6.mjs")), "this kind of diagram is not drawn here");
  assert.equal(why(new Error("No diagram type detected matching given configuration for text: nonsense")), "this kind of diagram is not drawn here");
  assert.equal(why(new Error("Parse error on line 2:\n...LR  A --> \n---------------------^\nExpecting 'AMP', got 'EOF'")), "Parse error on line 2: Expecting 'AMP', got 'EOF'");
  assert.equal(why(new Error("mermaid did not load")), "mermaid did not load");
  assert.equal(why("odd"), "odd");
  assert.equal(why(new Error("")), "mermaid failed");
  const long = why(new Error("x".repeat(500)));
  assert.equal(long.length, 160);
  assert.ok(long.endsWith("…"));
});

// The page draw() works in, one for the file: diagram.ts keeps what it drew,
// and watches the page, from its first diagram on.
const { document, look, observers, media } = install();

// A diagram element in the page, as markdown.ts draws a fence, under
// `parent`; with no <code> when `source` is null.
function fence(source, parent = document.body) {
  const el = document.createElement("div");
  el.className = "diagram";
  if (source !== null) {
    const code = document.createElement("code");
    code.textContent = source;
    el.append(code);
  }
  parent.append(el);
  return el;
}

const error = (el) => el.querySelector(".diagram-error")?.textContent;
const drawnSvg = (el) => el.shadowRoot.children[0];

// The tests below run in order: the first finds mermaid not yet loaded.

test("a page with no diagram imports nothing and watches nothing", () => {
  const p = document.createElement("p");
  document.body.append(p);
  draw(document.body);
  assert.equal(mermaid.imports, 0);
  assert.deepEqual(observers, []);
  assert.deepEqual(media, []);
  p.remove();
});

test("when mermaid does not load, the code block stays with a line that says so, and the next diagram imports it again", async () => {
  mermaid.fail = true;
  const first = fence("flowchart LR\n  A --> B");
  draw(document.body);
  await settle();
  assert.equal(mermaid.imports, 1);
  assert.equal(error(first), "not drawn: mermaid did not load");
  assert.equal(first.querySelector(".diagram-error").className, "meta diagram-error");
  assert.equal(first.shadowRoot, null);
  assert.ok(first.querySelector("code"), "the code block stays");
  mermaid.fail = false;
  // Whether a diagram that failed so is drawn once mermaid loads is not
  // pinned here: only that the import is tried again.
  first.remove();
  const second = fence("flowchart LR\n  C --> D");
  draw(document.body);
  await settle();
  assert.equal(mermaid.imports, 2, "the import is tried again");
  assert.ok(second.shadowRoot);
  assert.deepEqual(mermaid.renders.map((r) => r.source), ["flowchart LR\n  C --> D"]);
  draw(document.body);
  await settle();
  assert.equal(mermaid.imports, 2, "a loaded mermaid is imported once");
  assert.equal(mermaid.renders.length, 1, "a drawn diagram is not drawn again");
  second.remove();
});

test("mermaid is given the strict config in the page's look, on the colour behind the diagram", async () => {
  look.vars = { "--bg": "  #0d0f14 ", "--bg-1": "#151821", "--fg": "#eceef4", "--prose": " serif " };
  look.colorScheme = "dark";
  look.fontSize = "15px";
  const outer = document.createElement("section");
  outer.bg = "rgb(21, 22, 26)";
  const mid = document.createElement("div");
  mid.bg = "rgba(0, 0, 0, 0)";
  const inner = document.createElement("div");
  inner.bg = "transparent";
  outer.append(mid);
  mid.append(inner);
  document.body.append(outer);
  const onGround = fence("flowchart LR", inner);
  const bare = fence("flowchart TD");
  mermaid.configs = [];
  draw(document.body);
  await settle();
  const [c, d] = mermaid.configs;
  assert.equal(c.securityLevel, "strict");
  assert.equal(c.startOnLoad, false);
  assert.equal(c.htmlLabels, false);
  assert.equal(c.darkMode, true);
  assert.equal(c.fontFamily, "serif");
  assert.equal(c.themeVariables.fontSize, "15px");
  assert.equal(c.themeVariables.primaryColor, "#151821");
  assert.equal(c.themeVariables.background, "rgb(21, 22, 26)", "the first colour up the parents that is not transparent");
  assert.equal(d.themeVariables.background, "#0d0f14", "the page's --bg when no parent has one");
  assert.equal(d.themeVariables.edgeLabelBackground, "#0d0f14");
  assert.deepEqual(mermaid.renders.slice(-2).map((r) => r.source), ["flowchart LR", "flowchart TD"], "the configs are in the order of the diagrams");
  look.vars = { "--bg": " #0d0f14 ", "--fg": "#eceef4", "--prose": "serif" };
  look.colorScheme = "light";
  look.fontSize = "16px";
  outer.remove();
  bare.remove();
  assert.equal(onGround.isConnected, false);
});

test("each diagram is rendered by an id of its own, from its code block's text; one with no code block is not rendered", async () => {
  mermaid.renders = [];
  const a = fence("flowchart LR\n  A[a & b] --> B");
  const none = fence(null);
  const b = fence("sequenceDiagram");
  draw(document.body);
  await settle();
  assert.deepEqual(mermaid.renders.map((r) => r.source), ["flowchart LR\n  A[a & b] --> B", "sequenceDiagram"]);
  const ids = mermaid.renders.map((r) => r.id);
  assert.match(ids[0], /^diagram-\d+$/);
  assert.notEqual(ids[0], ids[1]);
  assert.equal(none.shadowRoot, null);
  assert.equal(error(none), undefined);
  for (const el of [a, none, b]) el.remove();
});

test("the SVG goes into an open shadow root: its <style> becomes a constructed sheet after the host's, and goes", async () => {
  mermaid.svg = () => '<svg id="m"><style>#m .node{fill:red}</style><g><style>#m text{fill:blue}</style><text>A</text></g></svg>';
  const el = fence("flowchart LR");
  draw(document.body);
  await settle();
  assert.equal(el.shadowRoot.mode, "open");
  const [host, sheet] = el.shadowRoot.adoptedStyleSheets;
  assert.match(host.text, /^:host \{ display: block;.*overflow-x: auto; \}\nsvg \{ display: block; height: auto; \}$/);
  assert.equal(sheet.text, "#m .node{fill:red}\n#m text{fill:blue}");
  assert.equal(el.shadowRoot.adoptedStyleSheets.length, 2);
  const svg = drawnSvg(el);
  assert.equal(el.shadowRoot.children.length, 1);
  assert.equal(svg.namespaceURI, SVG_NS);
  assert.equal(svg.ownerDocument, document, "the SVG is imported into the page's document");
  assert.deepEqual(svg.querySelectorAll("style"), [], "no <style> element is left");
  assert.equal(svg.querySelector("text").textContent, "A");
  assert.ok(el.querySelector("code"), "the code block stays in the light tree");
  el.remove();
});

test("every style attribute is set through the CSSOM and none is left as an attribute, a hostile one too", async () => {
  const hostile = "background:url(https://other.example/x.png);fill:red";
  mermaid.svg = () => `<svg style="max-width: 300px;"><g><rect style="${hostile}"/><rect class="plain"/><text style="fill:blue">A</text></g></svg>`;
  const el = fence("flowchart LR");
  draw(document.body);
  await settle();
  const svg = drawnSvg(el);
  const [styled, plain] = svg.querySelectorAll("rect");
  assert.equal(svg.style.cssText, "max-width: 300px;");
  assert.equal(styled.style.cssText, hostile);
  assert.equal(svg.querySelector("text").style.cssText, "fill:blue");
  assert.equal(plain.style.cssText, "");
  for (const e of [svg, ...svg.querySelectorAll("rect"), svg.querySelector("text")]) {
    assert.equal(e.hasAttribute("style"), false, e.localName);
  }
  el.remove();
});

test("a link in a diagram follows the page's rule: to a new tab when it may be followed, else no link at all", async () => {
  const cases = [
    // [attribute, value, href after or null]
    ["href", "https://other.example/a b", "https://other.example/a%20b"],
    ["href", "http://127.0.0.1:7337/c/abc", "http://127.0.0.1:7337/c/abc"],
    ["href", "mailto:ryan@example.com", "mailto:ryan@example.com"],
    ["href", "obsidian://open?vault=notes", "obsidian://open?vault=notes"],
    ["href", "javascript:alert(1)", null],
    ["href", " JaVa\tScript:alert(1)", null],
    ["href", "data:text/html,<b>x</b>", null],
    ["href", "vbscript:x", null],
    ["href", "file:///etc/passwd", null],
    ["href", "/c/abc", null],
    ["href", "#top", null],
    ["href", "", null],
    ["xlink:href", "https://other.example/x", "https://other.example/x"],
    ["xlink:href", "https://other.example/?a=1&b=2", "https://other.example/?a=1&b=2"],
    ["xlink:href", "javascript:alert(1)", null],
  ];
  mermaid.svg = () => `<svg>${cases.map(([name, value], i) => `<a ${name}="${value.replace(/&/g, "&amp;").replace(/"/g, "&quot;")}" target="_self" transform="translate(${i})"><text>${i}</text></a>`).join("")}</svg>`;
  const el = fence("flowchart LR");
  draw(document.body);
  await settle();
  const as = drawnSvg(el).querySelectorAll("a");
  assert.equal(as.length, cases.length, "every <a> stays, to keep its node's place");
  cases.forEach(([name, value, after], i) => {
    const a = as[i];
    const what = `${name}=${JSON.stringify(value)}`;
    assert.equal(a.getAttributeNS(XLINK_NS, "href"), null, `${what}: xlink:href goes`);
    assert.equal(a.getAttribute("transform"), `translate(${i})`, what);
    if (after) {
      assert.equal(a.getAttribute("href"), after, what);
      assert.equal(a.getAttribute("target"), "_blank", what);
      assert.equal(a.getAttribute("rel"), "noopener noreferrer", what);
    } else {
      assert.equal(a.getAttribute("href"), null, what);
      assert.equal(a.getAttribute("target"), null, what);
      assert.equal(a.getAttribute("rel"), null, what);
    }
  });
  el.remove();
});

test("a link's href is read before its xlink:href, and both go when the href may not be followed", async () => {
  mermaid.svg = () => '<svg><a href="javascript:alert(1)" xlink:href="https://other.example/"><text>x</text></a><a href="https://ok.example/" xlink:href="javascript:alert(1)"><text>y</text></a></svg>';
  const el = fence("flowchart LR");
  draw(document.body);
  await settle();
  const [bad, good] = drawnSvg(el).querySelectorAll("a");
  assert.equal(bad.getAttribute("href"), null);
  assert.equal(bad.getAttributeNS(XLINK_NS, "href"), null);
  assert.equal(good.getAttribute("href"), "https://ok.example/");
  assert.equal(good.getAttributeNS(XLINK_NS, "href"), null);
  el.remove();
});

test("text that does not parse stays the code block, with mermaid's first and last lines, and what mermaid left in the page goes", async () => {
  mermaid.svg = (id) => {
    for (const leftover of [id, `d${id}`]) {
      const e = document.createElement("div");
      e.setAttribute("id", leftover);
      document.body.append(e);
    }
    throw new Error("Parse error on line 2:\n...LR  A --> \n---------------------^\nExpecting 'AMP', got 'EOF'");
  };
  const el = fence("flowchart LR\n  A -->");
  draw(document.body);
  await settle();
  const id = mermaid.renders[mermaid.renders.length - 1].id;
  assert.equal(error(el), "not drawn: Parse error on line 2: Expecting 'AMP', got 'EOF'");
  assert.equal(el.shadowRoot, null);
  assert.equal(document.getElementById(id), null);
  assert.equal(document.getElementById(`d${id}`), null);
  el.remove();
});

test("a kind of diagram not vendored stays the code block, and says so", async () => {
  mermaid.svg = () => {
    throw new TypeError("Failed to fetch dynamically imported module: http://x/static/vendor/mermaid/chunks/ganttDiagram.mjs");
  };
  const el = fence("gantt\n  title x");
  draw(document.body);
  await settle();
  assert.equal(error(el), "not drawn: this kind of diagram is not drawn here");
  el.remove();
});

test("what mermaid gives that is not SVG is not drawn", async () => {
  for (const svg of ["<div>no</div>", "", "just text"]) {
    mermaid.svg = () => svg;
    const el = fence("flowchart LR");
    draw(document.body);
    await settle();
    assert.equal(error(el), "not drawn: mermaid drew no SVG", JSON.stringify(svg));
    assert.equal(el.shadowRoot, null);
    el.remove();
  }
});

test("a diagram taken out of the page before its turn is not rendered, and is drawn when it is back", async () => {
  mermaid.svg = () => "<svg><text>A</text></svg>";
  const before = mermaid.renders.length;
  const el = fence("flowchart LR");
  draw(document.body);
  el.remove();
  await settle();
  assert.equal(mermaid.renders.length, before);
  document.body.append(el);
  draw(document.body);
  await settle();
  assert.equal(mermaid.renders.length, before + 1, "it was forgotten, so it is drawn now");
  assert.ok(el.shadowRoot);
  el.remove();
});

test("a change of look or theme, or of the system's dark, draws each diagram again in the new colours, once each", async () => {
  assert.equal(observers.length, 1, "one watch, however many draws");
  const [{ target, options, callback }] = observers;
  assert.equal(target, document.documentElement);
  assert.deepEqual(options, { attributes: true, attributeFilter: ["data-look", "data-theme"] });
  assert.equal(media.length, 1);
  assert.equal(media[0].query, "(prefers-color-scheme: dark)");
  assert.equal(media[0].event, "change");
  for (const again of [callback, media[0].listener]) {
    let colour = 0;
    mermaid.svg = () => `<svg><text>${colour}</text></svg>`;
    look.vars = { "--bg": "#fff", "--bg-1": "#aaa" };
    const el = fence("flowchart LR");
    const gone = fence("flowchart TD");
    draw(document.body);
    await settle();
    const shadow = el.shadowRoot;
    gone.remove();
    const configs = mermaid.configs.length;
    const renders = mermaid.renders.length;
    colour = 1;
    look.vars = { "--bg": "#000", "--bg-1": "#123456" };
    again();
    again();
    await settle();
    assert.equal(mermaid.renders.length, renders + 1, "the diagram in the page, once; not the one taken out");
    assert.equal(mermaid.renders[mermaid.renders.length - 1].source, "flowchart LR");
    assert.equal(mermaid.configs[configs].themeVariables.primaryColor, "#123456");
    assert.equal(el.shadowRoot, shadow, "the same shadow root");
    assert.equal(el.shadowRoot.children.length, 1);
    assert.equal(drawnSvg(el).textContent, "1");
    again();
    await settle();
    assert.equal(mermaid.renders.length, renders + 2, "drawn again on the next change too");
    el.remove();
  }
});

test("a diagram that fails to draw again keeps the one it had, with no line", async () => {
  mermaid.svg = () => "<svg><text>kept</text></svg>";
  const el = fence("flowchart LR");
  draw(document.body);
  await settle();
  mermaid.svg = () => {
    throw new Error("Parse error");
  };
  observers[0].callback();
  await settle();
  assert.equal(drawnSvg(el).textContent, "kept");
  assert.equal(error(el), undefined);
  el.remove();
});

test("a diagram whose redraw failed is drawn again on the next change, in the new colours", async () => {
  let colour = "old";
  mermaid.svg = () => `<svg><text>${colour}</text></svg>`;
  const el = fence("flowchart LR");
  draw(document.body);
  await settle();
  mermaid.svg = () => {
    throw new Error("mermaid did not load");
  };
  observers[0].callback();
  await settle();
  assert.equal(drawnSvg(el).textContent, "old");
  colour = "new";
  mermaid.svg = () => `<svg><text>${colour}</text></svg>`;
  look.vars = { "--bg": "#000", "--bg-1": "#654321" };
  const configs = mermaid.configs.length;
  observers[0].callback();
  await settle();
  assert.equal(drawnSvg(el).textContent, "new");
  assert.equal(mermaid.configs[configs]?.themeVariables.primaryColor, "#654321");
  assert.equal(error(el), undefined);
  look.vars = { "--bg": " #0d0f14 ", "--fg": "#eceef4", "--prose": "serif" };
  el.remove();
});

test("text that does not parse is not tried again, on a draw or a change", async () => {
  mermaid.svg = () => {
    throw new Error("Parse error on line 1:\nExpecting 'AMP', got 'EOF'");
  };
  const el = fence("flowchart LR\n  A -->");
  draw(document.body);
  await settle();
  const renders = mermaid.renders.length;
  mermaid.svg = () => "<svg><text>A</text></svg>";
  draw(document.body);
  observers[0].callback();
  await settle();
  assert.equal(mermaid.renders.length, renders);
  assert.equal(el.querySelectorAll(".diagram-error").length, 1);
  assert.equal(el.shadowRoot, null);
  el.remove();
});
