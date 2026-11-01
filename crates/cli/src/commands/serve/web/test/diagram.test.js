import { test } from "node:test";
import assert from "node:assert/strict";
import { config, why } from "../static/diagram.ts";

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
