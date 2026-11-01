// Markdown for bodies, notes and outcomes (IN-02), drawn in the browser with
// marked. GFM tables are on. Raw HTML in the text is dropped, never drawn; a
// link follows the page's one rule for links (links.js), so a `javascript:`
// or relative target is shown as text; and links that leave the inbox open
// in a new tab. The page's policy blocks script even if something got
// through. A fenced block is a code block (code.js), and a ```diff or
// ```patch fence is drawn as a diff. A ```mermaid fence is a code block in
// an element of class "diagram", which diagram.js draws as its diagram.
import { Marked } from "marked";
import { codeBlock, wrapped } from "./code.js";
import { CASE_REF, JOINED, pageOf } from "./ids.js";
import { link } from "./links.js";

const marked = new Marked({ gfm: true, breaks: false, async: false });

marked.use({
  // marked takes the text after an inline <pre>, <code>, <kbd> or <script>
  // tag, up to its end tag, as raw HTML, to be drawn as it is. Raw HTML is
  // dropped here, so that text is text like any other.
  walkTokens(token) {
    if (token.type === "text") token.escaped = false;
  },
  renderer: {
    code(token) {
      const lang = (token.lang || "").split(/\s/)[0]!.toLowerCase();
      const text = token.text.endsWith("\n") ? token.text : `${token.text}\n`;
      if (lang === "mermaid") return `<div class="diagram"><div class="code">${codeBlock(text, false, wrapped())}</div></div>\n`;
      return `<div class="code">${codeBlock(text, lang === "diff" || lang === "patch", wrapped())}</div>\n`;
    },
    // Raw HTML, such as <b> or <script>, is left out.
    html() {
      return "";
    },
    // The link goes to the address the browser would follow, so what is
    // checked is what is followed.
    link(token) {
      const text = this.parser.parseInline(token.tokens);
      const url = target(token.href);
      if (!url) return text;
      let a = `<a href="${escape(url.href)}"`;
      if (token.title) a += ` title="${escape(token.title)}"`;
      if (url.origin !== location.origin) a += ' target="_blank" rel="noopener noreferrer"';
      return `${a}>${text}</a>`;
    },
    // A column's alignment is a class, which style.css aligns, as the Go
    // inbox drew it. So is a cell's length (`fit`): style.css keeps a short
    // cell on one line and gives a long one room to read.
    tablecell(token) {
      const text = this.parser.parseInline(token.tokens);
      const tag = token.header ? "th" : "td";
      const classes = [token.align && `align-${token.align}`, fit(token.text)].filter(Boolean);
      const attr = classes.length ? ` class="${classes.join(" ")}"` : "";
      return `<${tag}${attr}>${text}</${tag}>\n`;
    },
  },
});

// Whether a table cell of markdown `text` is short, as a number, a date, a
// host or a version is, and fits on one line: at most 24 characters in at
// most three words. Or long, as a sentence is, over 40 characters in at
// least five words, and wants a column wide enough to read. Or neither.
export function fit(text: string): "short" | "long" | "" {
  const t = text.trim();
  const words = t ? t.split(/\s+/).length : 0;
  if (t.length <= 24 && words <= 3) return "short";
  if (t.length > 40 && words >= 5) return "long";
  return "";
}

// A `case:ID` in the text links to that case's page in the inbox, which
// opens in the same tab, as the app's own links do; its text stays
// `case:ID`. Only text is read: code is not inline text, so a code span or
// block keeps its `case:ID`, and a link's text is left as it is. The
// case is not looked up: its page says when the store has no such case.
//
// A `case:ID` at the start of the text: nothing is before it in `src`, so
// the look back in CASE_REF passes, and the tokenizer checks the text
// before itself.
const CASE_REF_HERE = new RegExp(`^${CASE_REF.source}`, "u");
marked.use({
  extensions: [
    {
      name: "caseRef",
      level: "inline",
      // marked cuts the text before each place a `case:ID` might start.
      // search() leaves CASE_REF's lastIndex as it was: the regex is shared
      // (case.js), and a lastIndex left past 0 would make the next matchAll
      // skip the start of its text.
      start(src) {
        const at = src.search(CASE_REF);
        return at < 0 ? undefined : at;
      },
      // Here the text before is in the tokens so far, not in `src`: a
      // letter, digit or `_` at its end joins it to `case:`.
      tokenizer(src, tokens) {
        if (this.lexer.state.inLink || JOINED.test(tokens.at(-1)?.raw || "")) return undefined;
        const m = CASE_REF_HERE.exec(src);
        return m ? { type: "caseRef", raw: m[0], id: m[1] } : undefined;
      },
      renderer(token) {
        return `<a href="${escape(pageOf(token.id))}">case:${escape(token.id)}</a>`;
      },
    },
  ],
});

// The HTML of `text` as markdown.
export function markdown(text: string | undefined): string {
  return marked.parse(text || "") as string;
}

// The HTML of `text` as inline markdown, for a line such as an approval
// row's note: links, code and emphasis, drawn by the same marked, so by the
// same rules as markdown(), but no blocks, so a heading, list or table is its
// text.
export function inlineMarkdown(text: string | undefined): string {
  return marked.parseInline(text || "") as string;
}

// Where a link to `href` goes, if the page may link to it (links.js). The
// target is read as its href attribute holds it, character references
// decoded, as the browser would follow it.
function target(href: string): URL | null {
  return link(attribute(href));
}

// `href` as an href attribute holds it: marked leaves character references
// such as `&amp;` in a link's target, for the browser to decode as it reads
// the attribute. The element is never put in the page.
const reader = document.createElement("div");
function attribute(href: string): string {
  reader.innerHTML = `<a href="${href.replace(/"/g, "&quot;")}"></a>`;
  return (reader.firstChild as Element).getAttribute("href")!;
}

function escape(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}
