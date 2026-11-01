import { mock, test } from "node:test";
import assert from "node:assert/strict";
import { render } from "preact-render-to-string";
import { html } from "../static/html.ts";
import { codeBlock, copyCode, diffLines, isDiff } from "../static/code.ts";
import { markdown } from "../static/markdown.ts";
import { CaseView } from "../static/case.ts";

const kinds = (text) => diffLines(text).map((l) => l.kind);

const patch = [
  "diff --git a/pr-watch.md b/pr-watch.md",
  "index 3b18e51..a9c2f04 100644",
  "--- a/pr-watch.md",
  "+++ b/pr-watch.md",
  "@@ -655,4 +655,4 @@ intro",
  " ## Context management",
  "",
  "--- a rule, removed",
  "-This loop runs all day",
  "+++ a rule, added",
  "+This loop runs all day, fast",
  "\\ No newline at end of file",
  "",
].join("\n");

test("a unified diff's lines are known by their kind, a hunk's by its counts", () => {
  assert.deepEqual(kinds(patch), ["head", "head", "head", "head", "hunk", "ctx", "ctx", "del", "del", "add", "add", "hunk"]);
});

test("the lines are the text as written, each with its own newline", () => {
  for (const text of [patch, "no newline at the end", "-a\n\n+b\n", "", "\n\n"]) {
    assert.equal(diffLines(text).map((l) => l.text).join(""), text);
  }
});

test("outside a hunk, a line is known by its first character, and --- and +++ are a header only as a pair", () => {
  assert.deepEqual(kinds("- old\n+ new\n  same\n"), ["del", "add", "ctx"]);
  assert.deepEqual(kinds("--- a/x\n+++ b/x\n"), ["head", "head"]);
  assert.deepEqual(kinds("--- not a header\n-gone\n"), ["del", "del"]);
  assert.deepEqual(kinds("+++ not a header\n"), ["add"]);
  // A hunk with no counts has one line each side.
  assert.deepEqual(kinds("@@ -1 +1 @@\n-a\n+b\n+c\n"), ["hunk", "del", "add", "add"]);
});

test("a hunk whose counts run past its lines ends at the next header", () => {
  const text = "@@ -1,9 +1,9 @@\n-a\n+b\n@@ -20,2 +20,2 @@\n-c\n+d\ndiff --git a/y b/y\n--- a/y\n+++ b/y\n";
  assert.deepEqual(kinds(text), ["hunk", "del", "add", "hunk", "del", "add", "head", "head", "head"]);
});

test("a script is a diff when it starts with its file headers or diff --git", () => {
  assert.equal(isDiff("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n"), true);
  assert.equal(isDiff("--- a/x\r\n+++ b/x\r\n"), true);
  assert.equal(isDiff("diff --git a/x b/x\n"), true);
  assert.equal(isDiff("mv github/* notifications/github/"), false);
  assert.equal(isDiff("--- a/x\nrm -rf x\n"), false);
  assert.equal(isDiff("echo\n--- a/x\n+++ b/x\n"), false);
});

// The text a block's HTML shows, as its code: the tags gone, the
// references decoded.
function code(block) {
  const inner = /<code>([\s\S]*)<\/code>/.exec(block)[1];
  return inner.replace(/<[^>]*>/g, "").replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, '"').replace(/&amp;/g, "&");
}

test("a block's code is its text, escaped, with the copy and wrap buttons outside it", () => {
  const text = '-<b a="1">&amp;</b>\n+ok\n';
  for (const diff of [true, false]) {
    const block = codeBlock(text, diff, false);
    assert.equal(code(block), text);
    assert.equal(block.includes("<b "), false);
    assert.ok(block.indexOf('class="wrap-toggle"') < block.indexOf("<pre"));
    assert.ok(block.indexOf('class="copy-code"') < block.indexOf("<pre"));
    assert.match(block, /<button type="button" class="copy-code" aria-label="copy" title="copy"><svg [^>]*aria-hidden="true"/);
  }
  assert.match(codeBlock(text, true, false), /<pre class="diff"><code><span class="d-del">-&lt;b a=&quot;1&quot;&gt;&amp;amp;&lt;\/b&gt;\n<\/span><span class="d-add">\+ok\n<\/span><\/code><\/pre>/);
  assert.match(codeBlock(text, false, true), /aria-pressed="true"/);
  assert.match(codeBlock(text, false, false), /aria-pressed="false"/);
});

test("a diff or patch fence is drawn as a diff, and any other fence as code", () => {
  const diff = markdown("```diff\n- old\n+ new\n```\n");
  assert.match(diff, /<div class="code"><div class="code-tools"><button[^>]*class="copy-code"[\s\S]*?>wrap<\/button><\/div><pre class="diff"><code><span class="d-del">- old\n<\/span><span class="d-add">\+ new\n<\/span><\/code><\/pre><\/div>/);
  assert.match(markdown("```patch\n+x\n```\n"), /class="d-add"/);
  assert.match(markdown("```Diff title\n+x\n```\n"), /class="d-add"/);
  const sh = markdown("```sh\n- not a diff <x>\n```\n");
  assert.match(sh, /<div class="code"><div class="code-tools"><button[^>]*class="copy-code"[\s\S]*?>wrap<\/button><\/div><pre><code>- not a diff &lt;x&gt;\n<\/code><\/pre><\/div>/);
});

test("an approval row's script is drawn as a diff only when it is one", () => {
  const rows = [
    { id: "r1", label: "patch", link: "", script: "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n" },
    { id: "r2", label: "shell", link: "", script: "mv a b\n-x" },
  ];
  const c = { id: "aB3xQ", state: "open", kind: "approval", urgency: "today", title: "t", body: "", rows, opened_at: "2026-09-25T13:10:00Z", events: [], revision: 1, amend_seq: 0 };
  const page = render(html`<${CaseView} c=${c} now=${0} onSent=${() => {}} onCase=${() => {}} />`);
  const blocks = page.match(/<div class="code">[\s\S]*?<\/pre><\/div>/g);
  assert.equal(blocks.length, 2);
  assert.match(blocks[0], /<pre class="diff">/);
  assert.equal(code(blocks[0]), rows[0].script);
  assert.match(blocks[1], /<pre><code>mv a b\n-x<\/code><\/pre>/);
  for (const block of blocks) assert.match(block, /class="copy-code" aria-label="copy"/);
});

test("every kind of code block has a copy button", () => {
  for (const text of ["```\nplain\n```\n", "```sh\nls\n```\n", "```diff\n-a\n+b\n```\n", "```mermaid\nflowchart LR\n  A --> B\n```\n"]) {
    assert.equal(markdown(text).match(/class="copy-code"/g).length, 1, text);
  }
  assert.doesNotMatch(markdown("no code, only `inline`\n"), /copy-code/);
});

// A copy button in a block of `html`, as the page has it: its block's
// <code> holds the text the browser shows, and the button says what it did.
function button(html) {
  const diff = html.includes('<pre class="diff">');
  const text = { textContent: code(html), parentElement: { classList: { contains: (c) => c === "diff" && diff } } };
  const block = { querySelector: (s) => (s === "pre > code" ? text : null) };
  const attributes = {};
  return {
    code: text,
    attributes,
    textContent: "",
    innerHTML: "",
    closest: (s) => (s === ".code" ? block : null),
    setAttribute: (name, value) => { attributes[name] = value; },
  };
}

// Runs `f` with navigator.clipboard as `clipboard`, and a document and a
// selection that record what is selected, whose old copy command says
// `copies`. The clock is a mock, so the button's pause before it goes back
// keeps no test waiting.
const clipboard_ = [];
async function withPage(clipboard, f, copies = false) {
  clipboard_.splice(0);
  const saved = Object.getOwnPropertyDescriptor(globalThis, "navigator");
  const selected = [];
  mock.timers.enable({ apis: ["setTimeout"] });
  Object.defineProperty(globalThis, "navigator", { value: { clipboard }, configurable: true });
  const doc = globalThis.document;
  // The old copy command, as a browser runs it: a copy event whose
  // listeners may put their own text on the clipboard.
  const listeners = [];
  globalThis.document = {
    ...doc,
    createRange: () => ({ selectNodeContents(node) { this.node = node; } }),
    addEventListener: (type, f) => type === "copy" && listeners.push(f),
    removeEventListener: (type, f) => type === "copy" && listeners.splice(listeners.indexOf(f), 1),
    execCommand: (c) => {
      if (c !== "copy" || !copies) return false;
      const data = {};
      listeners.forEach((f) => f({ clipboardData: { setData: (t, v) => { data[t] = v; } }, preventDefault() {} }));
      clipboard_.push(data["text/plain"]);
      return true;
    },
  };
  globalThis.getSelection = () => ({ removeAllRanges: () => selected.splice(0), addRange: (r) => selected.push(r.node) });
  try {
    await f(selected);
  } finally {
    mock.timers.reset();
    if (saved) Object.defineProperty(globalThis, "navigator", saved);
    else delete globalThis.navigator;
    globalThis.document = doc;
    delete globalThis.getSelection;
  }
}

test("copy puts a block's text on the clipboard as written, a diff's + and - and all", async () => {
  const text = `${patch}\n`;
  const copied = [];
  await withPage({ writeText: async (t) => copied.push(t) }, async (selected) => {
    for (const diff of [true, false]) {
      const b = button(codeBlock(text, diff, false));
      assert.equal(await copyCode(b), "copied");
      assert.equal(b.textContent, "copied");
      assert.equal(b.attributes["aria-label"], "copied");
      assert.equal(b.attributes.title, "copied");
      mock.timers.tick(1500);
      assert.equal(b.attributes["aria-label"], "copy");
      assert.equal(b.attributes.title, "copy");
      assert.match(b.innerHTML, /^<svg /);
    }
    assert.deepEqual(selected, []);
  });
  // A diff keeps its last newline, as a patch needs it; any other block
  // goes without the one markdown.ts gives a fence, so a pasted command
  // does not run at once.
  assert.deepEqual(copied, [text, patch]);
});

test("a fence is copied without the newline markdown.ts adds to it", async () => {
  const copied = [];
  await withPage({ writeText: async (t) => copied.push(t) }, async () => {
    for (const fence of ["```sh\nrm -rf build\n```\n", "```\na\n  b\n```\n", "```diff\n-a\n+b\n```\n"]) {
      await copyCode(button(markdown(fence)));
    }
  });
  assert.deepEqual(copied, ["rm -rf build", "a\n  b", "-a\n+b\n"]);
});

test("where the page has no clipboard, or it refuses, copy selects the block's text and copies it the old way", async () => {
  for (const clipboard of [undefined, { writeText: async () => { throw new Error("denied"); } }]) {
    for (const copies of [true, false]) {
      await withPage(clipboard, async (selected) => {
        const b = button(codeBlock("ls -la\n", false, false));
        const done = copies ? "copied" : "selected";
        assert.equal(await copyCode(b), done);
        assert.deepEqual(selected, [b.code]);
        assert.equal(b.textContent, done);
        // The old command copies the block's text, not the selection's.
        assert.deepEqual(clipboard_, copies ? ["ls -la"] : []);
      }, copies);
    }
  }
});
