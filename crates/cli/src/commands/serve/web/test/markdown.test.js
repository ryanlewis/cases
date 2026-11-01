import { test } from "node:test";
import assert from "node:assert/strict";
import { fit, inlineMarkdown, markdown } from "../static/markdown.ts";

test("a table cell is short, long or neither by its length and its words", () => {
  for (const text of ["host-a", "3.1.4", "2026-09-26 08:15", "Things task", "obsidian-sync (`ob`)", "", "✓"]) {
    assert.equal(fit(text), "short", text);
  }
  for (const text of ["Book sync goes via sync.example.com, LAN only", "stays on the SSH agent; keep a config backup"]) {
    assert.equal(fit(text), "long", text);
  }
  // Long without words to break between, as a URL or a hash is, or a few
  // words just past short: neither.
  for (const text of ["https://github.com/prometheus/prometheus/releases", "node 22 + obsidian-headless", "a b c d"]) {
    assert.equal(fit(text), "", text);
  }
});

test("a table cell carries its column's alignment and its length as classes", () => {
  const html = markdown("| Host | Notes | Size |\n|:---|---|---:|\n| host-a | Book sync goes via sync.example.com, LAN only | node 22 + obsidian-headless |\n");
  assert.match(html, /<th class="align-left short">Host<\/th>/);
  assert.match(html, /<th class="short">Notes<\/th>/);
  assert.match(html, /<td class="align-left short">host-a<\/td>/);
  assert.match(html, /<td class="long">Book sync goes via sync.example.com, LAN only<\/td>/);
  assert.match(html, /<td class="align-right">node 22 \+ obsidian-headless<\/td>/);
});

test("a mermaid fence is its code block in a diagram element, and no other fence is", () => {
  const diagram = markdown("```mermaid\nflowchart LR\n  A[<b>x</b>] --> B\n```\n");
  assert.match(diagram, /^<div class="diagram"><div class="code"><div class="code-tools"><button[^>]*class="copy-code"[\s\S]*?>wrap<\/button><\/div><pre><code>flowchart LR\n  A\[&lt;b&gt;x&lt;\/b&gt;\] --&gt; B\n<\/code><\/pre><\/div><\/div>\n$/);
  assert.match(markdown("```Mermaid title\ngraph TD\n```\n"), /^<div class="diagram">/);
  for (const text of ["```\nflowchart LR\n```\n", "```js\nmermaid()\n```\n", "```diff\n+x\n```\n", "    flowchart LR\n", "mermaid\n", "`mermaid`\n"]) {
    assert.doesNotMatch(markdown(text), /diagram/, text);
  }
});

test("inline markdown makes the same links and images as the body, by the same rules", () => {
  const tags = (html) => html.match(/<(a|img)\b[^>]*>/g) || [];
  for (const text of [
    "[PR](https://github.com/o/r/pull/1)", "<https://x.test/a>", "<mailto:a@b.test>", "a@b.test", "www.x.test/p", "https://x.test/p?a=1&b=2",
    "<javascript:alert(1)>", "javascript:alert(1)", "[a](&#106;avascript:alert(1))", "[a](java&#x09;script:x)", "[x](<javascript:alert(1)>)",
    "[a](../land.sh)", "[a](/cases/x)", '<a href="https://x.test">x</a>', "<img src=x onerror=alert(1)>", "![x](https://x.test/i.png)",
  ]) {
    assert.deepEqual(tags(inlineMarkdown(text)), tags(markdown(text)), text);
  }
});

test("inline markdown draws no blocks, and a link definition in it defines nothing", () => {
  assert.equal(inlineMarkdown("# a\n- b\n> c"), "# a\n- b\n&gt; c");
  assert.equal(inlineMarkdown("[r]\n\n[r]: javascript:x"), "[r]\n\n[r]: javascript:x");
});

// The links to cases in `html`, as [href, text].
const caseLinks = (html) => [...html.matchAll(/<a href="([^"]*)">([^<]*)<\/a>/g)].map((m) => [m[1], m[2]]);

test("a case:ID in text links to its case's page in the inbox, in the same tab, and keeps its text", () => {
  assert.equal(markdown("see case:eba6l"), '<p>see <a href="/cases/eba6l">case:eba6l</a></p>\n');
  assert.equal(inlineMarkdown("case:AbC12"), '<a href="/cases/AbC12">case:AbC12</a>');
  // In a heading, a list, a table and emphasis, as in any text.
  for (const text of ["# about case:eba6l", "- case:eba6l", "| a |\n|---|\n| case:eba6l |", "**case:eba6l**", "*x*case:eba6l"]) {
    assert.deepEqual(caseLinks(markdown(text)), [["/cases/eba6l", "case:eba6l"]], text);
  }
});

test("a case:ID ends at punctuation or a dash, which stay text", () => {
  assert.equal(markdown("(case:eba6l), case:AbC12. case:x1y2z!"), '<p>(<a href="/cases/eba6l">case:eba6l</a>), <a href="/cases/AbC12">case:AbC12</a>. <a href="/cases/x1y2z">case:x1y2z</a>!</p>\n');
  // An id holds no dash, so an address's slug after it is text.
  assert.equal(inlineMarkdown("case:UkLWZ-pin-bun"), '<a href="/cases/UkLWZ">case:UkLWZ</a>-pin-bun');
});

test("case: joined to a word, or with no id after it, links nothing", () => {
  for (const text of ["showcase:abcde", "mycase:abcde", "x_case:abcde", "a\\_case:abcde", "case:abcd", "case:abcde_x", "case:abcdé", "case:abcdeé", "écase:abcde", "日case:abcde", "case: abcde", "Case:abcde", "CASE:abcde", "case:", "case:ab-cdefg"]) {
    assert.deepEqual(caseLinks(markdown(text)), [], text);
    assert.deepEqual(caseLinks(inlineMarkdown(text)), [], text);
  }
});

test("a case:ID in code, or in a link's text or address, is left as it is", () => {
  assert.equal(inlineMarkdown("`case:eba6l`"), "<code>case:eba6l</code>");
  assert.match(markdown("```\ncase:eba6l\n```\n"), /<pre><code>case:eba6l\n<\/code><\/pre>/);
  assert.match(markdown("    case:eba6l\n"), /<pre><code>case:eba6l\n<\/code><\/pre>/);
  assert.equal(inlineMarkdown("[see case:eba6l](https://x.test/)"), '<a href="https://x.test/" target="_blank" rel="noopener noreferrer">see case:eba6l</a>');
  assert.equal(inlineMarkdown("https://x.test/case:eba6l"), '<a href="https://x.test/case:eba6l" target="_blank" rel="noopener noreferrer">https://x.test/case:eba6l</a>');
  assert.equal(inlineMarkdown("<https://x.test/case:eba6l>"), '<a href="https://x.test/case:eba6l" target="_blank" rel="noopener noreferrer">https://x.test/case:eba6l</a>');
});
