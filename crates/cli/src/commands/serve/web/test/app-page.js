// A stand-in page for the inbox's App (static/app.ts), as much of a browser
// as App reads: the address and its history, the document's listeners and
// the few elements App asks it for, the event stream, the clock, and serve
// behind fetch. App is mounted with hooks-stand-in.js (through
// app-stand-in.js), so a test calls it as a function, runs its effects, and
// draws the tree it gives to HTML with render-to-string to see the page.
//
// This module must be the first to load app.ts in its process, as `node
// --test` gives each file one: it puts the stand-ins in place, then loads
// app.ts, whose start() hands the stand-in render the App to mount.
import { mock } from "node:test";
import { registerHooks } from "node:module";
import { Fragment, h } from "preact";
import { render } from "preact-render-to-string";

const standIn = new URL("./app-stand-in.js", import.meta.url).href;
const APP = /\/static\/app\.ts$/;
registerHooks({
  resolve(specifier, context, next) {
    const parent = context.parentURL ?? "";
    if ((specifier === "preact" || specifier === "preact/hooks") && APP.test(parent)) return next(standIn, context);
    return next(specifier, context);
  },
});

export const ORIGIN = "http://127.0.0.1:7337";
// The clock at mount: 13:45, as test/case.js's cases have it.
export const NOW = Date.parse("2026-09-25T13:45:00Z");
// The build the page says it is, and serve's unless a test says otherwise.
export const BUILD = "b1";
// The two lists App reads, as their requests name them.
export const WAITING = "GET /v1/cases?state=open,parked";
export const OTHERS = "GET /v1/cases?state=answered,pickedup,closed,withdrawn,sent";
export const LISTENERS = "GET /v1/listeners";
export const INDEX = "GET /v1";

// The document hooks.js set up, for its createElement, which markdown.ts
// uses while a case is drawn.
const base = globalThis.document;

// Every vnode of `tree`, in order.
export function* nodes(tree) {
  if (Array.isArray(tree)) {
    for (const child of tree) yield* nodes(child);
    return;
  }
  if (!tree || typeof tree !== "object") return;
  yield tree;
  yield* nodes(tree.props?.children);
}

// An element of the page: it answers `closest` from `closer`, and
// `querySelector(All)` from `q`, each keyed by selector.
export function element(props = {}) {
  return {
    tagName: "DIV",
    attrs: {},
    scrollTop: 0,
    closer: {},
    q: {},
    hasAttribute(name) {
      return name in this.attrs;
    },
    getAttribute(name) {
      return name in this.attrs ? this.attrs[name] : null;
    },
    setAttribute(name, value) {
      this.attrs[name] = String(value);
    },
    closest(selector) {
      return this.closer[selector] ?? null;
    },
    querySelector(selector) {
      return (this.q[selector] || [])[0] || null;
    },
    querySelectorAll(selector) {
      return this.q[selector] || [];
    },
    matches(selector) {
      return (this.is || []).includes(selector);
    },
    ...props,
  };
}

// A link to `path` as the page draws one: `target`, `download`, or in the
// composer's dialog.
export function link(path, { target = "", download = false, composer = false } = {}) {
  const a = element({ tagName: "A", href: new URL(path, ORIGIN).href, target });
  if (download) a.attrs.download = "";
  a.closer["a[href]"] = a;
  if (composer) a.closer["#compose"] = element();
  return a;
}

// A form of the page that typed() reads: one matching `is` (selectors),
// holding `fields` (text fields and their values) and `checked` (inputs
// checked, each `{ preset }` when only the options pre-selected it).
export function form({ is = [], fields = [], checked = [], edited = false } = {}) {
  const f = element({ tagName: "FORM", is });
  if (edited) f.attrs["data-edited"] = "";
  f.q["input:checked"] = checked.map(({ preset = false } = {}) => element({ tagName: "INPUT", closer: preset ? { "[data-preset]": element() } : {} }));
  f.q['textarea, input[type="text"]:not([name="other"]):not([name="filter"])'] = fields.map((value) => element({ tagName: "TEXTAREA", value }));
  return f;
}

// Listeners by event type, as a document and a window keep them.
function listening() {
  const by = new Map();
  return {
    addEventListener(type, fn) {
      if (!by.has(type)) by.set(type, new Set());
      by.get(type).add(fn);
    },
    removeEventListener(type, fn) {
      by.get(type)?.delete(fn);
    },
    dispatch(type, e) {
      for (const fn of [...(by.get(type) || [])]) fn(e);
    },
    count(type) {
      return by.get(type)?.size ?? 0;
    },
  };
}

// An event as the browser would hand a listener: `preventDefault` marks it.
function event(props) {
  return {
    button: 0,
    metaKey: false,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    repeat: false,
    isComposing: false,
    defaultPrevented: false,
    preventDefault() {
      this.defaultPrevented = true;
    },
    ...props,
  };
}

// serve, behind fetch: GET /v1, the two lists, a case, the listeners and the
// notifications, from what `s` holds. `s.routes[request]`, a function of
// the request, answers it in place of serve: with `{ status, body }`, a
// promise of one, or by throwing as fetch does when serve is out of reach.
// `s.down` makes every request throw.
function serve(page) {
  return async (url, init = {}) => {
    const request = `${init.method || "GET"} ${decodeURIComponent(url)}`;
    page.requests.push(request);
    const s = page.server;
    if (s.down) throw new TypeError("fetch failed");
    const route = s.routes[request];
    const { status, body } = route ? await route(request, init) : answer(s, request);
    return { ok: status >= 200 && status < 300, status, json: async () => (body === undefined ? JSON.parse("") : structuredClone(body)) };
  };
}

function answer(s, request) {
  const path = request.replace(/^GET /, "");
  if (path === "/v1") return { status: 200, body: { version: s.build, csrf: "token" } };
  if (request === WAITING) return { status: 200, body: { list: s.waiting } };
  if (request === OTHERS) return { status: 200, body: { list: s.done } };
  if (path === "/v1/listeners") return { status: 200, body: { list: s.listeners } };
  if (path.startsWith("/v1/notifications")) return { status: 200, body: s.notifications };
  const one = /^\/v1\/cases\/(.+)$/.exec(path);
  if (one) {
    const c = s.cases[one[1]] || [...s.waiting, ...s.done].find((c) => c.id === one[1]);
    return c ? { status: 200, body: c } : { status: 404, body: { error: { category: "not_found", message: `no case ${one[1]}` } } };
  }
  return { status: 404, body: { error: { category: "request", message: `no route ${request}` } } };
}

// The page: its globals, put in place at once, and what was done to them.
// `url` is the address it was loaded at.
function setUp({ url = "/", server = {}, prefs = {}, meta = BUILD } = {}) {
  const page = {
    requests: [],
    // What sent the page away: assign, replace or reload, with the path.
    left: [],
    // What App did to the history and the scroll.
    log: [],
    streams: [],
    prefs: { ...prefs },
    server: { build: BUILD, waiting: [], done: [], cases: {}, listeners: [], routes: {}, down: false, notifications: { boot: "boot1", latest: 0, items: [] }, ...server },
  };
  const entries = [{ state: null, url: new URL(url, ORIGIN).href }];
  let at = 0;
  const pathOf = (href) => {
    const u = new URL(href);
    return u.pathname + u.search + u.hash;
  };
  const history = {
    scrollRestoration: "auto",
    get state() {
      return entries[at].state;
    },
    pushState(state, _unused, to) {
      entries.splice(at + 1);
      entries.push({ state: structuredClone(state), url: to == null ? entries[at].url : new URL(to, entries[at].url).href });
      at++;
      page.log.push(`push ${pathOf(entries[at].url)}`);
    },
    replaceState(state, _unused, to) {
      entries[at] = { state: structuredClone(state), url: to == null ? entries[at].url : new URL(to, entries[at].url).href };
      page.log.push(`replace ${pathOf(entries[at].url)}`);
    },
    // A move in the history lands in a task of its own, with popstate, as
    // the browser's does.
    go(by) {
      const to = at + by;
      if (to < 0 || to >= entries.length) return;
      setImmediate(() => {
        at = to;
        page.window.dispatch("popstate", { state: entries[at].state });
      });
    },
    back() {
      this.go(-1);
    },
    forward() {
      this.go(1);
    },
  };
  page.history = history;
  page.entries = () => entries.map((e, i) => `${i === at ? ">" : ""}${pathOf(e.url)}`);
  page.location = {
    origin: ORIGIN,
    get href() {
      return entries[at].url;
    },
    get pathname() {
      return new URL(entries[at].url).pathname;
    },
    get search() {
      return new URL(entries[at].url).search;
    },
    get hash() {
      return new URL(entries[at].url).hash;
    },
    assign: (path) => page.left.push(`assign ${path}`),
    replace: (path) => page.left.push(`replace ${path}`),
    reload: () => page.left.push("reload"),
  };
  page.list = element({ scrollTop: 0 });
  page.work = element({ scrollTop: 0 });
  page.favicon = element({ attrs: { href: "/icon.svg" } });
  page.found = {};
  page.drawn = "";
  const doc = {
    ...listening(),
    title: "",
    hidden: false,
    body: element({ tagName: "BODY" }),
    createElement: base.createElement,
    querySelector(selector) {
      if (selector === 'meta[name="cases-build"]') return meta === null ? null : { content: meta };
      if (selector === ".split > .list") return page.list;
      if (selector === ".split > .work") return page.work;
      // What the 15 s poll looks for, in the page as last drawn.
      if (selector === ".zero-panel, #compose") return /class="zero-panel"|id="compose"/.test(page.drawn) ? element() : null;
      return (page.found[selector] || [])[0] || null;
    },
    querySelectorAll(selector) {
      return page.found[selector] || [];
    },
    getElementById(id) {
      return id === "favicon" ? page.favicon : null;
    },
  };
  page.document = doc;
  page.window = {
    ...listening(),
    scrollY: 0,
    scrollTo(x, y) {
      this.scrollY = y;
      page.log.push(`scrollTo ${y}`);
    },
    focus() {},
    casesPrefs: sharedPrefs,
    EventSource: class {
      constructor(url) {
        this.url = url;
        this.closed = false;
        page.streams.push(this);
      }
      close() {
        this.closed = true;
      }
    },
  };
  page.stored = new Map();
  current = page;
  Object.assign(globalThis, { history, location: page.location, document: doc, window: page.window, fetch: serve(page), EventSource: page.window.EventSource });
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem: (k) => (page.stored.has(k) ? page.stored.get(k) : null),
      setItem: (k, v) => page.stored.set(k, String(v)),
      removeItem: (k) => page.stored.delete(k),
    },
  });
  return page;
}

// options.ts takes window.casesPrefs once, as it loads: it reads the page's
// preferences through this, whichever page is up.
let current = null;
const sharedPrefs = { read: (name) => current?.prefs[name] ?? "" };

// app.ts, loaded on a first page: what start() did, and App. A test file
// that sets globalThis.firstPage before it loads this module has that page
// (setUp's options) be the first: app-start.test.js loads app.ts on a page
// with no build in it.
const firstPage = setUp(globalThis.firstPage);
const { rendered } = await import("./app-stand-in.js");
const { mount } = await import("./hooks-stand-in.js");
const { Options, Keys } = await import("../static/options.ts");
const { Page } = await import("../static/page.ts");
const { Composer } = await import("../static/compose.ts");
await import("../static/app.ts");
// start() may wait on serve for its build before it draws.
for (let i = 0; i < 5 && !rendered.length; i++) await new Promise((r) => setImmediate(r));
if (!rendered.length) throw new Error("app.ts's start() drew nothing within 5 turns of the event loop");
const App = rendered[0].vnode.type;
export const first = { build: rendered[0].vnode.props.build, scrollRestoration: history.scrollRestoration, parent: rendered[0].parent, body: firstPage.document.body, requests: firstPage.requests };

// App, mounted on a page loaded at `url` (see setUp), its clock at `now`
// (NOW unless given) and its timers the test's to move. `extra(page)`
// runs before the mount, to add what the page has beyond the plain one.
// Gives the page with:
// - tree, html: what App gave last, and that drawn
// - settle(): lets what App started run, and draws again
// - tick(ms): moves the clock
// - click(el, mods), key(key, props): an event on the document
// - back(), forward(): the browser's buttons
// - stream: the event stream open last
// - find(type): the vnodes of a component in the tree
export async function openApp({ build = BUILD, extra, now = NOW, ...options } = {}) {
  mock.timers.reset();
  mock.timers.enable({ apis: ["setTimeout", "setInterval", "Date"], now });
  const page = setUp(options);
  page.dialogs = { options: dialog(page, "options"), keys: dialog(page, "keys") };
  extra?.(page);
  const attach = (tree) => {
    for (const v of nodes(tree)) {
      if (v.type === Options) v.props.dialog.current = page.dialogs.options;
      if (v.type === Keys) v.props.dialog.current = page.dialogs.keys;
    }
  };
  const app = mount(App, { build }, attach);
  // As descriptors, so that the getters stay getters.
  Object.defineProperties(page, Object.getOwnPropertyDescriptors({
    app,
    get tree() {
      return app.tree;
    },
    async settle() {
      for (let i = 0; i < 12; i++) {
        await new Promise((r) => setImmediate(r));
        app.draw();
      }
      page.drawn = render(h(Fragment, null, app.tree));
      return page;
    },
    get html() {
      return page.drawn;
    },
    async tick(ms) {
      mock.timers.tick(ms);
      return page.settle();
    },
    async click(target, mods = {}) {
      const e = event({ target, ...mods });
      page.document.dispatch("click", e);
      await page.settle();
      return e.defaultPrevented;
    },
    async key(key, props = {}) {
      const e = event({ key, target: page.document.body, ...props });
      page.document.dispatch("keydown", e);
      await page.settle();
      return e.defaultPrevented;
    },
    async back() {
      history.back();
      return page.settle();
    },
    async forward() {
      history.forward();
      return page.settle();
    },
    get stream() {
      return page.streams.at(-1);
    },
    find(type) {
      return [...nodes(app.tree)].filter((v) => v.type === type);
    },
    get page() {
      return page.find(Page)[0];
    },
    get composer() {
      return page.find(Composer)[0];
    },
    get recorded() {
      return [...nodes(app.tree)].find((v) => v.props && v.props.recorded);
    },
    // The requests made since the last call.
    taken() {
      return page.requests.splice(0);
    },
  }));
  await page.settle();
  return page;
}

// A dialog's stand-in: showModal is logged.
function dialog(page, name) {
  return {
    showModal() {
      page.log.push(`showModal ${name}`);
    },
  };
}

// A card of the list, for j and k: a link to `path`, the page's own when
// `current`, whose click is the browser's, a click event on the document.
export function card(page, path, current = false) {
  const a = link(path);
  if (current) a.attrs["aria-current"] = "page";
  a.click = () => page.document.dispatch("click", event({ target: a }));
  return a;
}
