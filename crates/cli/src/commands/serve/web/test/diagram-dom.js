// A stand-in for the browser and for mermaid, enough for diagram.ts: a small
// DOM (elements in namespaces, attributes, a shadow root, the few selectors
// diagram.ts asks for), DOMParser for the SVG mermaid gives, CSSStyleSheet,
// getComputedStyle, MutationObserver and matchMedia, and a mermaid that gives
// what the test tells it to. Nothing here checks how a browser would treat
// the SVG: the tests pin what diagram.ts does with it.
import { registerHooks } from "node:module";

export const SVG_NS = "http://www.w3.org/2000/svg";
export const XLINK_NS = "http://www.w3.org/1999/xlink";
const HTML_NS = "http://www.w3.org/1999/xhtml";

// mermaid as the test sets it: `svg(id, source)` gives the SVG or throws;
// `fail` makes the import itself fail. `imports` counts the imports tried,
// `configs` and `renders` record what diagram.ts passed.
export const mermaid = { svg: () => "<svg></svg>", fail: false, imports: 0, configs: [], renders: [] };
globalThis.__mermaid = mermaid;

const STUB = "data:text/javascript," + encodeURIComponent(`
const m = globalThis.__mermaid;
export default {
  initialize(config) { m.configs.push(config); },
  async render(id, source) { m.renders.push({ id, source }); return { svg: await m.svg(id, source) }; },
};`);

registerHooks({
  resolve(specifier, context, next) {
    if (specifier !== "mermaid") return next(specifier, context);
    mermaid.imports += 1;
    if (mermaid.fail) throw new Error("Failed to fetch dynamically imported module: mermaid");
    // A new URL each import, so that one that failed is not remembered.
    return { url: `${STUB}%2F%2F${mermaid.imports}`, shortCircuit: true };
  },
});

export class Node {
  constructor(ns, name, doc) {
    this.namespaceURI = ns;
    this.localName = name;
    this.ownerDocument = doc;
    this.attributes = [];
    this.children = [];
    this.parentElement = null;
    this.host = null;
    this.text = "";
    this.style = { cssText: "" };
    this.shadowRoot = null;
    this.bg = "";
  }
  attr(ns, name) {
    return this.attributes.find((a) => a.ns === ns && a.local === name);
  }
  getAttribute(name) {
    const a = this.attributes.find((a) => (a.prefix ? `${a.prefix}:${a.local}` : a.local) === name);
    return a ? a.value : null;
  }
  getAttributeNS(ns, name) {
    const a = this.attr(ns, name);
    return a ? a.value : null;
  }
  hasAttribute(name) {
    return this.getAttribute(name) !== null;
  }
  setAttribute(name, value) {
    const a = this.attributes.find((a) => !a.prefix && a.local === name);
    if (a) a.value = String(value);
    else this.attributes.push({ ns: null, prefix: "", local: name, value: String(value) });
  }
  setAttributeNS(ns, qualified, value) {
    const [prefix, local] = qualified.includes(":") ? qualified.split(":") : ["", qualified];
    this.removeAttributeNS(ns, local);
    this.attributes.push({ ns, prefix, local, value: String(value) });
  }
  removeAttribute(name) {
    this.attributes = this.attributes.filter((a) => (a.prefix ? `${a.prefix}:${a.local}` : a.local) !== name);
  }
  removeAttributeNS(ns, name) {
    this.attributes = this.attributes.filter((a) => !(a.ns === ns && a.local === name));
  }
  set className(value) {
    this.setAttribute("class", value);
  }
  get className() {
    return this.getAttribute("class") || "";
  }
  get id() {
    return this.getAttribute("id") || "";
  }
  get textContent() {
    return this.text + this.children.map((c) => c.textContent).join("");
  }
  set textContent(value) {
    this.children = [];
    this.text = value;
  }
  get firstElementChild() {
    return this.children[0] || null;
  }
  get isConnected() {
    let e = this;
    while (e.parentElement || e.host) e = e.parentElement || e.host;
    return e === this.ownerDocument.documentElement;
  }
  append(...nodes) {
    for (const n of nodes) {
      n.remove();
      n.parentElement = this;
      this.children.push(n);
    }
  }
  replaceChildren(...nodes) {
    for (const c of this.children) c.parentElement = null;
    this.children = [];
    this.append(...nodes);
  }
  remove() {
    if (!this.parentElement) return;
    const p = this.parentElement;
    p.children = p.children.filter((c) => c !== this);
    this.parentElement = null;
  }
  attachShadow({ mode }) {
    if (this.shadowRoot) throw new Error("already a shadow root");
    const root = new Node(null, "#shadow-root", this.ownerDocument);
    root.mode = mode;
    root.host = this;
    root.adoptedStyleSheets = [];
    this.shadowRoot = root;
    return root;
  }
  matches(selector) {
    if (selector.startsWith(".")) return this.className.split(/\s+/).includes(selector.slice(1));
    if (selector.startsWith("[")) return this.hasAttribute(selector.slice(1, -1));
    return this.localName === selector;
  }
  querySelectorAll(selector) {
    const found = [];
    const walk = (e) => {
      for (const c of e.children) {
        if (c.matches(selector)) found.push(c);
        walk(c);
      }
    };
    walk(this);
    return found;
  }
  querySelector(selector) {
    return this.querySelectorAll(selector)[0] || null;
  }
  clone(doc) {
    const n = new Node(this.namespaceURI, this.localName, doc);
    n.attributes = this.attributes.map((a) => ({ ...a }));
    n.text = this.text;
    for (const c of this.children) n.append(c.clone(doc));
    return n;
  }
}

// A document: <html> with a <body>, as the page has.
export function page() {
  const doc = {};
  doc.documentElement = new Node(HTML_NS, "html", doc);
  doc.body = new Node(HTML_NS, "body", doc);
  doc.documentElement.append(doc.body);
  doc.createElement = (name) => new Node(HTML_NS, name, doc);
  doc.importNode = (node, deep) => (deep ? node.clone(doc) : new Node(node.namespaceURI, node.localName, doc));
  doc.getElementById = (id) => {
    const walk = (e) => {
      for (const c of e.children) {
        if (c.id === id) return c;
        const f = walk(c);
        if (f) return f;
      }
      return null;
    };
    return walk(doc.documentElement);
  };
  return doc;
}

// Reads `html` as markup of elements and text, the SVG in the SVG
// namespace, xlink:href in xlink's: no more than the SVG the tests give.
function parse(html, doc) {
  const body = new Node(HTML_NS, "body", doc);
  const open = [body];
  const tag = /<\/?([a-zA-Z][\w-]*)((?:\s+[\w:-]+(?:="[^"]*")?)*)\s*(\/?)>/g;
  let at = 0;
  for (let m; (m = tag.exec(html)); ) {
    const top = open[open.length - 1];
    top.text += m.index > at && top.children.length === 0 ? html.slice(at, m.index) : "";
    at = tag.lastIndex;
    if (m[0].startsWith("</")) {
      open.pop();
      continue;
    }
    const svg = m[1] === "svg" || top.namespaceURI === SVG_NS;
    const e = new Node(svg ? SVG_NS : HTML_NS, m[1], doc);
    for (const [, name, value = ""] of m[2].matchAll(/([\w:-]+)(?:="([^"]*)")?/g)) {
      if (name === "xlink:href") e.setAttributeNS(XLINK_NS, name, value);
      else e.setAttribute(name, value.replace(/&quot;/g, '"').replace(/&amp;/g, "&"));
    }
    top.append(e);
    if (!m[3]) open.push(e);
  }
  return body;
}

// The stand-in browser, as globals, with `look` as the page's computed
// styles: its custom properties, colour scheme and text size. `observers`
// and `media` hold what diagram.ts watches.
export function install() {
  const doc = page();
  const look = { vars: { "--bg": " #0d0f14 ", "--fg": "#eceef4", "--prose": "serif" }, colorScheme: "light", fontSize: "16px" };
  const observers = [];
  const media = [];
  globalThis.document = doc;
  globalThis.DOMParser = class {
    parseFromString(text, type) {
      if (type !== "text/html") throw new Error(type);
      return { body: parse(text, page()) };
    }
  };
  globalThis.CSSStyleSheet = class {
    replaceSync(text) {
      this.text = text;
    }
  };
  globalThis.getComputedStyle = (e) => ({
    backgroundColor: e.bg,
    getPropertyValue: (name) => (e === doc.documentElement ? look.vars[name] ?? "" : ""),
    colorScheme: e === doc.documentElement ? look.colorScheme : "",
    fontSize: e === doc.body ? look.fontSize : "",
  });
  globalThis.MutationObserver = class {
    constructor(callback) {
      this.callback = callback;
    }
    observe(target, options) {
      observers.push({ target, options, callback: this.callback });
    }
  };
  globalThis.window = {
    matchMedia: (query) => ({ addEventListener: (event, listener) => media.push({ query, event, listener }) }),
  };
  return { document: doc, look, observers, media };
}

// Lets the queue of renders run out.
export async function settle() {
  for (let i = 0; i < 20; i++) await new Promise((r) => setImmediate(r));
}
