// Diagrams (IN-02): a ```mermaid fence, drawn by the vendored mermaid
// (vendor/README.md). markdown.ts draws the fence as a code block in an
// element of class "diagram"; draw() swaps each for its diagram. mermaid is
// over a megabyte, so it is imported only when a page has a diagram.
//
// Only flowchart, sequence, class, state and ER diagrams are vendored. Any
// other kind, or text that does not parse, stays the code block it was,
// with a line under it that says why.
//
// The text is an agent's, so mermaid runs at its strict level: no script,
// no click handlers, and its own sanitising of labels, which are SVG text,
// never HTML. The page's policy allows no inline style, so the diagram's
// stylesheet is a constructed one, and its style attributes are set through
// the CSSOM, in a shadow root of the element that keeps the page's styles
// out. The policy still blocks script and anything from another server. A
// link in a diagram follows the page's one rule for links (links.js).
//
// A diagram takes its colours and face from the page's look and theme, and
// is drawn again when either changes.
import { link } from "./links.js";
import type { Mermaid, MermaidConfig } from "mermaid";

// The layout at the diagram element: its space, and a wide diagram that
// scrolls sideways rather than pushing the page wider.
const HOST = `:host { display: block; margin: .75rem 0; overflow-x: auto; }
svg { display: block; height: auto; }`;

const SVG_NS = "http://www.w3.org/2000/svg";
const XLINK_NS = "http://www.w3.org/1999/xlink";

// Config keys a diagram's own directives may not change: mermaid's own,
// and those that would draw labels as HTML, loosen the sanitising of
// labels, lay out with a library not vendored, or override the page's
// theme.
const SECURE = [
  "secure", "securityLevel", "startOnLoad", "maxTextSize", "suppressErrorRendering", "maxEdges",
  "htmlLabels", "flowchart", "class", "state", "layout",
  "theme", "themeVariables", "themeCSS", "darkMode", "fontFamily",
  "dompurifyConfig",
];

let loading: Promise<Mermaid> | null = null;
// Renders run one at a time: mermaid's config is global.
let queue: Promise<unknown> = Promise.resolve();
let count = 0;
let watching = false;
// The source of each diagram drawn, to draw it again in a new theme.
const drawn = new Map<Element, string>();
// The diagrams waiting in the queue: one wait each, which draws in the
// theme the page has when its turn comes.
const queued = new Set<Element>();
// The diagrams not drawn as mermaid did not load: the next draw tries them
// again, where one whose text does not parse is not.
const unloaded = new WeakSet<Element>();

// mermaid did not load: not the diagram's fault, and worth another try.
class Unloaded extends Error {}

// Draw each diagram under `root` that is not drawn yet. A root without one
// costs a query and nothing is imported.
export function draw(root: Element): void {
  // Forget the diagrams no longer in the page, such as another case's.
  for (const el of drawn.keys()) if (!el.isConnected) drawn.delete(el);
  const found = [...root.querySelectorAll(".diagram")].filter((el) => !drawn.has(el) && (unloaded.has(el) || !el.querySelector(".diagram-error")));
  if (!found.length) return;
  watch();
  for (const el of found) {
    const code = el.querySelector("code");
    if (code) render(el, code.textContent || "");
  }
}

// mermaid, imported once; an import that fails is tried again next time,
// and says so rather than blame the kind of diagram.
function mermaid(): Promise<Mermaid> {
  loading ??= import("mermaid").then(
    (m) => m.default,
    () => {
      loading = null;
      throw new Unloaded("mermaid did not load");
    },
  );
  return loading;
}

// The config for the look and theme the page has now: mermaid's base theme
// in the page's colours and face, on `ground`, the colour behind the
// diagram, which backs its edges' labels. The text is the page's size:
// mermaid measures its labels in the page, where its own style is blocked,
// so a size of its own would draw them wider than it measured.
export function config(css: { getPropertyValue(name: string): string; colorScheme?: string; fontSize?: string }, ground: string): MermaidConfig {
  const v = (name: string) => css.getPropertyValue(name).trim();
  const font = v("--prose");
  return {
    startOnLoad: false,
    securityLevel: "strict",
    suppressErrorRendering: true,
    secure: SECURE,
    htmlLabels: false,
    flowchart: { htmlLabels: false },
    layout: "dagre",
    look: "classic",
    theme: "base",
    darkMode: css.colorScheme === "dark",
    fontFamily: font,
    themeVariables: {
      darkMode: css.colorScheme === "dark",
      fontFamily: font,
      fontSize: css.fontSize || "16px",
      background: ground,
      primaryColor: v("--bg-1"),
      primaryTextColor: v("--fg"),
      primaryBorderColor: v("--rule-strong"),
      secondaryColor: v("--bg-2"),
      tertiaryColor: v("--bg"),
      lineColor: v("--fg-2"),
      textColor: v("--fg"),
      edgeLabelBackground: ground,
      noteBkgColor: v("--sun-bg"),
      noteTextColor: v("--sun-fg"),
      noteBorderColor: v("--sun-edge"),
    },
  };
}

function render(el: Element, source: string): void {
  drawn.set(el, source);
  if (queued.has(el)) return;
  queued.add(el);
  queue = queue.then(async () => {
    queued.delete(el);
    if (!el.isConnected) {
      drawn.delete(el);
      return;
    }
    const id = `diagram-${++count}`;
    try {
      const m = await mermaid();
      const root = getComputedStyle(document.documentElement);
      const body = getComputedStyle(document.body);
      m.initialize(config({ getPropertyValue: (name) => root.getPropertyValue(name), colorScheme: root.colorScheme, fontSize: body.fontSize }, ground(el)));
      const { svg } = await m.render(id, source);
      show(el, svg);
    } catch (e) {
      document.getElementById(id)?.remove();
      document.getElementById(`d${id}`)?.remove();
      fail(el, e);
    }
  });
}

// The colour behind `el`: the first background of it or a parent's that is
// not transparent.
function ground(el: Element): string {
  for (let e: Element | null = el; e; e = e.parentElement) {
    const color = getComputedStyle(e).backgroundColor;
    if (color && color !== "transparent" && color !== "rgba(0, 0, 0, 0)") return color;
  }
  return getComputedStyle(document.documentElement).getPropertyValue("--bg").trim();
}

// The diagram in `el`'s shadow root: its <style> as a constructed sheet,
// its style attributes through the CSSOM, and its links by the page's rule.
function show(el: Element, svg: string): void {
  // Read as the page reads HTML, in a document of its own that loads
  // nothing and runs nothing: mermaid's SVG is not always well-formed XML.
  const parsed = new DOMParser().parseFromString(svg, "text/html").body.firstElementChild;
  if (!parsed || parsed.namespaceURI !== SVG_NS) throw new Error("mermaid drew no SVG");
  const sheet = new CSSStyleSheet();
  sheet.replaceSync([...parsed.querySelectorAll("style")].map((s) => (s.remove(), s.textContent)).join("\n"));
  const host = new CSSStyleSheet();
  host.replaceSync(HOST);
  const node = document.importNode(parsed, true);
  const from = [parsed, ...parsed.querySelectorAll("[style]")].filter((e) => e.hasAttribute("style"));
  const to = [node, ...node.querySelectorAll("[style]")].filter((e) => e.hasAttribute("style"));
  to.forEach((e, i) => {
    e.removeAttribute("style");
    (e as SVGElement).style.cssText = from[i]!.getAttribute("style")!;
  });
  for (const a of node.querySelectorAll("a")) links(a);
  const shadow = el.shadowRoot || el.attachShadow({ mode: "open" });
  shadow.adoptedStyleSheets = [host, sheet];
  shadow.replaceChildren(node);
  unloaded.delete(el);
  el.querySelector(".diagram-error")?.remove();
}

// A link in a diagram: to a new tab when the page may link to its target,
// and no link when not. The <a> stays, without an href, which is no link:
// mermaid places the node by the <a>'s transform.
function links(a: Element): void {
  const href = a.getAttribute("href") ?? a.getAttributeNS(XLINK_NS, "href") ?? "";
  const url = link(href);
  a.removeAttributeNS(XLINK_NS, "href");
  if (!url) {
    a.removeAttribute("href");
    a.removeAttribute("target");
    return;
  }
  a.setAttribute("href", url.href);
  a.setAttribute("target", "_blank");
  a.setAttribute("rel", "noopener noreferrer");
}

// A diagram that did not draw: the code block stays, with why under it. One
// drawn before keeps what it drew, and is drawn again on the next change.
function fail(el: Element, e: unknown): void {
  if (el.shadowRoot) return;
  drawn.delete(el);
  if (e instanceof Unloaded) unloaded.add(el);
  el.querySelector(".diagram-error")?.remove();
  const line = document.createElement("p");
  line.className = "meta diagram-error";
  line.textContent = `not drawn: ${why(e)}`;
  el.append(line);
}

// Why a diagram did not draw, in a line: a kind not vendored, or mermaid's
// first and last lines, which say where the text fails and what it wanted.
export function why(e: unknown): string {
  const message = e instanceof Error ? e.message : String(e);
  if (/dynamically imported module|Failed to fetch|No diagram type detected/i.test(message)) {
    return "this kind of diagram is not drawn here";
  }
  const lines = message.split("\n").map((l) => l.trim()).filter(Boolean);
  const text = lines.length > 1 ? `${lines[0]} ${lines[lines.length - 1]}` : lines[0] || "mermaid failed";
  return text.length > 160 ? `${text.slice(0, 159)}…` : text;
}

// Draw each diagram again when the look or theme changes, or the system's
// light or dark does.
function watch(): void {
  if (watching) return;
  watching = true;
  const again = () => {
    for (const [el, source] of drawn) {
      if (el.isConnected) render(el, source);
      else drawn.delete(el);
    }
  };
  new MutationObserver(again).observe(document.documentElement, { attributes: true, attributeFilter: ["data-look", "data-theme"] });
  window.matchMedia?.("(prefers-color-scheme: dark)").addEventListener("change", again);
}
