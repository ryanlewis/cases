// The inbox: a single-page app over serve's API (spec/api.md). The page
// paths are / and /cases/{id}, each under a filter, and /done, the done
// filter's old address (IN-01, IN-02, IN-07; see routes.ts); the app draws
// them all, and moves between them without loading the page again. The
// composer's address, /message, opens the composer over the list.
import { render } from "preact";
import { useEffect, useLayoutEffect, useRef, useState } from "preact/hooks";
import * as api from "./api.js";
import { html } from "./html.js";
import { page } from "./ids.js";
import { keys } from "./keys.js";
import { copyCode } from "./code.js";
import { Composer, Entry } from "./compose.js";
import { DONE, Tally, addressee, afterRead, icon, listed, split, tally } from "./lists.js";
import { Keys, Options, notify } from "./options.js";
import { Page, withRefused } from "./page.js";
import { moved, query, same, under, where } from "./routes.js";
import type { What } from "./api.js";
import type { Data, Shown } from "./page.js";
import type { Route } from "./routes.js";
import type { Case, Listener } from "./types.js";

// A write the page made, for the line that says so, and who a message went
// to.
interface Sent {
  event: What;
  id: string;
  title: string;
  at: number;
  to: string;
}

// The composer open over the page: who it is for and what about, as its
// link gave them, and when it opened, which draws it afresh each time.
interface Composing {
  to: string;
  re: string;
  at: number;
}

// The page a route shows, and the composer it opens: the composer's
// address is the list under its filter, with the composer open over it, and
// the address bar says the list's, so that a reload does not open it again.
function arrive(r: Route): { route: Route; composing: Composing | null } {
  if (r.page !== "compose") return { route: r, composing: null };
  const route: Route = { page: "list", show: r.show || "inbox" };
  history.replaceState(history.state, "", `/${query(route.show!)}`);
  return { route, composing: { to: r.to || "", re: r.re || "", at: Date.now() } };
}

// Where a page and its two columns were scrolled.
interface Scroll {
  y: number;
  list: number;
  work: number;
}

// The states the inbox lists.
const WAITING = ["open", "parked"];

// The inbox under its filter, where an address is none of the app's pages.
const HOME: Route = { page: "list", show: "inbox" };

// What the page's address shows. /done is put in the history as the
// address of the filter it is, so the address bar and a reload say it.
function here(): Route {
  const to = moved(location);
  if (to) history.replaceState(history.state, "", to);
  return where(location) || HOME;
}

// A form on the page holds something typed or chosen, which a reload would
// lose: the case's answer, or its withdraw's reason, or a withdraw's reason
// on an inbox-zero row, the edit of a message, or a message being written.
// A verdict the options only pre-selected (case.ts) is not one chosen; one
// given by a key is. The edit of a message is drawn holding the message's
// body, so it holds typed text only once that differs, as the form says.
// The composer's choice of who a message goes to, alone, is not worth
// keeping, even a name the address filled in, nor is the filter of its
// choices. The options and keys dialogs keep nothing a reload would lose.
function typed(): boolean {
  const forms = Array.from(document.querySelectorAll("#case form.respond, #compose form, form.row-confirm"));
  return forms.some((form) => {
    if (form.matches('[data-write="amend"]')) return form.hasAttribute("data-edited");
    const chosen = Array.from(form.querySelectorAll("input:checked")).some((input) => !input.closest("[data-preset]"));
    if (form.matches("#case form") && chosen) return true;
    const fields = form.querySelectorAll<HTMLTextAreaElement | HTMLInputElement>('textarea, input[type="text"]:not([name="other"]):not([name="filter"])');
    return Array.from(fields).some((t) => t.value.trim());
  });
}

// Where the page and its two columns are scrolled to.
function scrolled(): Scroll {
  const list = document.querySelector<HTMLElement>(".split > .list");
  const work = document.querySelector<HTMLElement>(".split > .work");
  return { y: window.scrollY, list: list ? list.scrollTop : 0, work: work ? work.scrollTop : 0 };
}

// An open composer has its entry in the history, which back takes off.
// The page under it keeps where it was scrolled, as a move by a link
// does, for a back to it after "about" takes the entry's place.
function useComposerEntry(composing: Composing | null, entry: Entry): void {
  useEffect(() => {
    if (!composing) return;
    if (!entry.on && !(history.state && history.state.composer)) history.replaceState({ ...history.state, scrolled: scrolled() }, "");
    entry.opened({ to: composing.to, re: composing.re });
  }, [composing]);
}

// The title leads with the open cases, and the tab icon shows them (IN-01).
// Set as the page is drawn, not after it is painted: a hidden tab paints
// late or not at all, and its title is what shows it has changed.
function useTitle(data: Data | null): void {
  useLayoutEffect(() => {
    if (!data) return;
    const t = tally(data.inbox);
    document.title = t.waiting ? `(${t.waiting}) cases` : "cases";
    const link = document.getElementById("favicon");
    const href = icon(t);
    // Chromium loads the new icon when the link's address changes.
    if (link && link.getAttribute("href") !== href) link.setAttribute("href", href);
  }, [data]);
}

// A move to another page starts at its top, and a move back by the back
// or forward button returns to where the page was scrolled.
function useScroll(data: Data | null, pinned: { current: Route | null }, restore: { current: Scroll | null }): void {
  const shownRoute = data && data.route;
  useEffect(() => {
    if (!shownRoute || shownRoute === pinned.current) return;
    const to = restore.current;
    restore.current = null;
    window.scrollTo(0, to ? to.y : 0);
    const list = document.querySelector<HTMLElement>(".split > .list");
    const work = document.querySelector<HTMLElement>(".split > .work");
    if (to && list) list.scrollTop = to.list;
    if (work) work.scrollTop = to ? to.work : 0;
  }, [shownRoute]);
}

// What the page's links and the back and forward buttons move it with
// (useLinks): the composer's entry in the history and what back does to it,
// the route shown, whether serve is a newer build, where a page returned to
// was scrolled, the keys dialog, and App's moves.
interface Links {
  entry: Entry;
  composerBack: { current: (() => boolean) | null };
  routeRef: { current: Route };
  stale: { current: boolean };
  restore: { current: Scroll | null };
  keysDialog: { current: HTMLDialogElement | null };
  navigate: (path: string, fromComposer?: boolean) => void;
  shut: () => void;
  setComposing: (composing: Composing | null) => void;
  setRoute: (route: Route) => void;
}

// Links to the app's pages move within it; the browser's back and
// forward buttons too, back to where each page was scrolled.
// An effect of App's, in its place, run once with what App's first draw
// gave it.
function useLinks({ entry, composerBack, routeRef, stale, restore, keysDialog, navigate, shut, setComposing, setRoute }: Links): void {
  useEffect(() => {
    const click = (e: MouseEvent) => {
      if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
      const a = (e.target as Element).closest && (e.target as Element).closest<HTMLAnchorElement>("a[href]");
      if (!a || a.target || a.hasAttribute("download")) return;
      const url = new URL(a.href, location.href);
      if (url.origin !== location.origin || !where(url)) return;
      // A link within the page shown, such as #thread, is the browser's.
      if (url.pathname === location.pathname && url.search === location.search && a.href.includes("#")) return;
      e.preventDefault();
      navigate(url.pathname + url.search, a.closest("#compose") !== null);
    };
    // A move within the page shown, to a fragment and back, keeps it. A
    // move to another page loads a newer build, as a link does.
    const back = () => {
      const took = entry.popped();
      const state = history.state;
      const { route: at, composing: opens } = arrive(here());
      // A case's page a refusal pinned keeps its filter in its entry, as its
      // address cannot name the inbox's (refused).
      const r: Route = state && state.show ? { ...at, show: state.show } : at;
      if (took) {
        // Back took the composer's entry off: the composer answers it, as
        // it does Escape. While it asks about a note, the entry is back
        // on. One not drawn yet, as the page has not been read, goes. A
        // back past the page under it, as a jump from the back button's
        // list, moves the page too, below.
        const answer = composerBack.current;
        if (!answer) shut();
        else if (answer()) entry.kept();
      } else if (opens) {
        setComposing(opens);
      } else if (state && state.composer && !entry.on) {
        // Forward to the entry of a composer closed before opens it
        // again, rather than being a place that back leaves at once.
        entry.adopted(state.composer);
        setComposing({ to: state.composer.to || "", re: state.composer.re || "", at: Date.now() });
      }
      if (same(r, routeRef.current)) return;
      if (stale.current) {
        location.reload();
        return;
      }
      restore.current = state && state.scrolled;
      setRoute(r);
    };
    // A code block's copy button (code.js).
    const copy = (e: MouseEvent) => {
      const button = (e.target as Element).closest && (e.target as Element).closest<HTMLElement>(".copy-code");
      if (button) void copyCode(button);
    };
    document.addEventListener("click", click);
    document.addEventListener("click", copy);
    window.addEventListener("popstate", back);
    keys(() => keysDialog.current);
    notify(navigate);
    return () => {
      document.removeEventListener("click", click);
      document.removeEventListener("click", copy);
      window.removeEventListener("popstate", back);
    };
  }, []);
}

// What the page follows the store with (useLive): the store's version it
// last read at, a read of what it shows, a check of serve's build, who it
// takes to be listening after a read of them, and its clock and data.
interface Live {
  version: { current: string };
  refresh: () => Promise<void>;
  checkBuild: () => Promise<void>;
  heard: (read: Listener[] | null) => Listener[] | null;
  setNow: (now: number) => void;
  setData: (update: (was: Data | null) => Data | null) => void;
  reopen: { current: (() => boolean) | null };
  failed: { current: boolean };
}

// The page following the store: the event stream, the minute's tick and
// the 15 s read of who is listening, each an effect of App's, in App's
// order. Each runs once, with what App's first draw gave it.
function useLive({ version, refresh, checkBuild, heard, setNow, setData, reopen, failed }: Live): void {
  // The event stream (IN-06). One tab of the browser holds it, shown or
  // hidden: a browser allows six connections to one address, shared by all
  // its tabs, and each stream holds one. The holder is the tab that holds
  // the "cases-stream" lock; it passes each version to the other tabs, which
  // read the store at once too. When the holder goes, the lock passes to
  // another tab, which opens its stream from the last version it had, which
  // serve answers at once if the store has moved since. So a hidden tab
  // follows the store, and its title with it, as a shown one does. Where
  // the browser has no locks, only a tab that is shown holds a stream, as
  // before.
  useEffect(() => {
    const channel = "BroadcastChannel" in window ? new BroadcastChannel("cases-store") : null;
    const locks = channel && "locks" in navigator ? navigator.locks : null;
    let stream: EventSource | null = null;
    let dropped = false;
    // Whether the stream opening is one the minute's tick opened again,
    // which checked the build itself.
    let reopened = false;
    let gone = false;
    let release: (() => void) | null = null;
    const seen = (v: string) => {
      if (v === version.current) return;
      version.current = v;
      refresh();
    };
    const open = () => {
      if (stream || !window.EventSource || (channel && !locks && document.hidden)) return;
      const after = version.current ? `?after=${encodeURIComponent(version.current)}` : "";
      stream = new EventSource(`/v1/changes${after}`);
      stream.onmessage = (e) => {
        if (channel) channel.postMessage(e.data);
        seen(e.data);
      };
      stream.onerror = () => (dropped = true);
      // Whenever the stream opens, serve may be another build: it may have
      // been upgraded while this tab held no stream, or while the stream was
      // down (IN-12). A read set off as the stream dropped, as when serve
      // restarts, may have failed with it: read again.
      stream.onopen = () => {
        // One the minute's tick opened again: the tick checks the build.
        if (reopened && !dropped) return;
        checkBuild();
        if (dropped) refresh();
        // The other tabs hold no stream of their own: they check the build
        // too, and read again after a drop.
        if (channel && locks) channel.postMessage({ opened: true, dropped });
        dropped = false;
      };
    };
    const close = () => {
      if (stream) stream.close();
      stream = null;
      reopened = false;
    };
    reopen.current = () => {
      if (!stream || dropped) return false;
      close();
      reopened = true;
      open();
      return true;
    };
    if (channel) {
      channel.onmessage = (e) => {
        if (typeof e.data === "string") return seen(e.data);
        checkBuild();
        if (e.data && e.data.dropped) refresh();
      };
    }
    const visibility = () => (document.hidden ? close() : open());
    if (locks) {
      // Held until this page goes: the browser lets it go with the page.
      locks
        .request("cases-stream", () => {
          if (gone) return undefined;
          open();
          return new Promise<void>((resolve) => (release = resolve));
        })
        .catch(() => {});
    } else {
      document.addEventListener("visibilitychange", visibility);
      open();
    }
    return () => {
      gone = true;
      close();
      if (release) release();
      document.removeEventListener("visibilitychange", visibility);
      if (channel) channel.close();
    };
  }, []);

  // Ages stay right, and a lost message is caught up, once a minute. So is
  // an upgrade, for a browser whose stream did not come back after serve
  // restarted (IN-12). A tab whose stream is up opens it again rather than
  // read every case: serve answers a stream opened after the version the
  // page has at once if the store has moved since, which the page then
  // reads, so a stream that died without a word loses nothing. It reads who
  // is listening, which moves no version (ST-16). A tab with no stream up,
  // or whose last read failed, reads the store.
  useEffect(() => {
    const timer = setInterval(() => {
      setNow(Date.now());
      if (failed.current || !(reopen.current && reopen.current())) refresh();
      else
        void api.listeners().then((read) => {
          const listeners = heard(read);
          setData((was) => (was ? { ...was, listeners } : was));
        });
      checkBuild();
    }, 60000);
    return () => clearInterval(timer);
  }, []);

  // Who is listening changes with no event in the store, as each wait
  // refreshes its record (ST-16): while the page shows it, on the inbox-zero
  // panel or in the composer, it is read again every 15 s.
  useEffect(() => {
    const timer = setInterval(async () => {
      if (!document.querySelector(".zero-panel, #compose")) return;
      const listeners = heard(await api.listeners());
      setNow(Date.now());
      setData((was) => (was ? { ...was, listeners } : was));
    }, 15000);
    return () => clearInterval(timer);
  }, []);
}

function App({ build }: { build: string }) {
  const [first] = useState(() => arrive(here()));
  const [route, setRoute] = useState<Route>(first.route);
  const [composing, setComposing] = useState<Composing | null>(first.composing);
  // The composer's entry in the history, and what back does to it (Entry).
  const [entry] = useState(() => new Entry(history));
  const composerBack = useRef<(() => boolean) | null>(null);
  // The composer open now, so that the close of one it replaced, which
  // lands after, does not close it.
  const composingNow = useRef(composing);
  composingNow.current = composing;
  // The composer goes: what back does goes with it.
  const shut = () => {
    composerBack.current = null;
    setComposing(null);
  };
  // What the page shows, for the route it was read for: the filter the list
  // is under, the open and parked cases, the others, and the case shown.
  const [data, setData] = useState<Data | null>(null);
  const [problem, setProblem] = useState("");
  const [recorded, setRecorded] = useState<Sent | null>(null);
  // Set when serve is a newer build while something is typed: the page says
  // so until the human reloads or moves to another page, which loads the
  // newer build (IN-12).
  const [newer, setNewer] = useState(false);
  const [now, setNow] = useState(Date.now());
  const [, redraw] = useState(0);
  const routeRef = useRef(route);
  routeRef.current = route;
  const loads = useRef(0);
  // The route of the last page drawn, to tell a move from a refresh.
  const shownOnce = useRef<Route | null>(null);
  // The store's version the page last read at, from the event stream.
  const version = useRef("");
  // Set when serve is a newer build than the page: the page loads it at the
  // next point where nothing typed would be lost (IN-12).
  const stale = useRef(false);
  // Whether a check of serve's build is out, whether another was asked for
  // while it was, and whether one has set off a reload: see checkBuild.
  const checking = useRef(false);
  const again = useRef(false);
  const reloading = useRef(false);
  // Opens this tab's stream again, from the version the page has, and says
  // whether it did: only a tab whose stream is up does (the minute's tick).
  const reopen = useRef<(() => boolean) | null>(null);
  // Whether the last read of the store failed: the minute's tick reads it
  // again, as the stream has nothing more to say until the store moves.
  const failed = useRef(false);
  // Who the page takes to be listening, and how many reads of the listeners
  // in a row have failed: the page's load and the 15 s poll both read them.
  const listening = useRef<{ list: Listener[] | null; missed: number }>({ list: null, missed: 0 });
  // Who the page takes to be listening after a read of the listeners that
  // gave `read` (afterRead).
  function heard(read: Listener[] | null): Listener[] | null {
    const [list, missed] = afterRead(read, listening.current.list, listening.current.missed);
    listening.current = { list, missed };
    return list;
  }
  const optionsDialog = useRef<HTMLDialogElement>(null);
  const keysDialog = useRef<HTMLDialogElement>(null);
  // The case shown on the list's page, and the route it was shown for: a
  // refresh keeps it while it is under the filter, with what is typed in
  // its form (IN-06), and a move to the list picks the first case again
  // (IN-01).
  const home = useRef<{ route: Route; id: string } | null>(null);
  // The route a refusal on the list pinned the page to: the same page, which
  // neither cross-fades nor scrolls to the top.
  const pinned = useRef<Route | null>(null);
  // Where a page the back or forward button returns to was scrolled.
  const restore = useRef<Scroll | null>(null);

  // Reads what `r` shows. A read that a later one overtakes is dropped.
  async function load(r: Route): Promise<void> {
    const seq = ++loads.current;
    try {
      const [waiting, others, one, read] = await Promise.all([
        api.list(WAITING),
        api.list(DONE),
        r.page === "case" ? api.one(r.id!) : null,
        api.listeners(),
      ]);
      const listeners = heard(read);
      // Briefings go in their own tab (IN-14).
      const { inbox, done, briefings } = split(waiting, others);
      let shown: Shown | null = null;
      if (one && one.ok) shown = { id: r.id!, case: one.body };
      else if (one && one.status === 404) shown = { id: r.id!, missing: true };
      else if (one) throw new api.Refused(one);
      // A case's page without a filter lists the case under its own.
      const filter = r.show || (shown && shown.case ? under(shown.case) : "inbox");
      if (r.page === "list") {
        const cases = listed(inbox, done, filter, Date.now(), briefings);
        const kept = home.current && home.current.route === r ? cases.find((c) => c.id === home.current!.id) : null;
        const first = kept || cases[0];
        if (first) shown = { id: first.id, case: first };
      }
      if (seq !== loads.current) return;
      home.current = r.page === "list" && shown ? { route: r, id: shown.id } : null;
      const show = () => {
        // A read overtaken while its cross-fade waited is dropped too.
        if (seq !== loads.current) return;
        setData({ route: r, show: filter, inbox, done, briefings, shown, listeners });
        setProblem("");
        failed.current = false;
      };
      // A move to another page cross-fades, as the stylesheet's view
      // transition names ask, where the browser can and motion is not
      // reduced; a refresh of the page shown changes at once. The new page
      // is drawn in a task after the state changes.
      if (r !== shownOnce.current && document.startViewTransition && !matchMedia("(prefers-reduced-motion: reduce)").matches) {
        document.startViewTransition(() => new Promise((drawn) => {
          try {
            show();
          } finally {
            setTimeout(drawn, 0);
          }
        }));
      } else {
        show();
      }
      shownOnce.current = r;
    } catch (e) {
      if (seq !== loads.current) return;
      failed.current = true;
      // serve answered, and says why it could not read; or it did not.
      setProblem(e instanceof api.Refused ? e.message : "serve cannot be reached. the inbox follows the store again once it is back.");
    }
  }
  const refresh = () => load(routeRef.current);

  // Goes to `path`, or loads it as a page when serve is a newer build. The
  // page left keeps where it was scrolled, for the back button. The
  // composer's address opens it over the page shown, which stays.
  function navigate(path: string, fromComposer = false): void {
    const to = where(new URL(path, location.href));
    if (to && to.page === "compose") {
      setComposing({ to: to.to || "", re: to.re || "", at: Date.now() });
      return;
    }
    if (stale.current) {
      // Loaded in the place of the composer's entry, which would otherwise
      // be left behind as a back that does nothing.
      if (entry.on) location.replace(path);
      else location.assign(path);
      return;
    }
    // While the composer's entry is on, the page moved to takes its place.
    // A link in the composer, to the case it is about, closes it; another
    // move, as a notification's, leaves it open over the new page, with
    // what is written in it, and an entry of its own again.
    if (entry.moved(path)) {
      if (fromComposer) shut();
      else entry.kept();
      setRoute(here());
      return;
    }
    history.replaceState({ ...history.state, scrolled: scrolled() }, "");
    history.pushState(null, "", path);
    setRoute(here());
  }

  // Asks serve which build it is: when the event stream opens, as serve may
  // have been upgraded while the stream was away, and once a minute, for a
  // stream that did not come back. One asks at a time, and none once the
  // page is reloading. One asked for while another is out runs after it:
  // the one out may have reached serve before a restart.
  async function checkBuild(): Promise<void> {
    if (reloading.current) return;
    if (checking.current) {
      again.current = true;
      return;
    }
    checking.current = true;
    again.current = false;
    try {
      const index = await api.index();
      const next = api.upgrade(index.version, build, typed());
      if (next === "same") return;
      stale.current = true;
      if (next === "say") {
        setNewer(true);
        return;
      }
      reloading.current = true;
      location.reload();
    } catch (e) {
      // serve cannot be reached: the stream or the tick asks again.
    } finally {
      checking.current = false;
      if (again.current) checkBuild();
    }
  }

  useEffect(() => {
    load(route);
  }, [route]);

  useComposerEntry(composing, entry);

  useLive({ version, refresh, checkBuild, heard, setNow, setData, reopen, failed });

  useLinks({ entry, composerBack, routeRef, stale, restore, keysDialog, navigate, shut, setComposing, setRoute });

  useTitle(data);

  useScroll(data, pinned, restore);

  // A write was made: say so once, and go to the next case in the list as
  // it stood before the send, in the order it draws them, or to the list
  // after the last (IN-03), under the same filter. A write that comes back
  // after the human moved to another page only says so: `data` is what the
  // page showed when the send was made.
  function sent(what: What, after: Case): void {
    setRecorded({ event: what, id: after.id, title: after.title, at: Date.now(), to: what === "message" ? addressee(after) : "" });
    // A message sent leaves the page where it was: the composer closes over
    // it, and the line says so.
    if (what === "message") return;
    if (!data || routeRef.current !== data.route) return;
    // An edit of a message stays on it: the stream draws it as edited.
    if (what === "amend") return;
    // A write from the inbox-zero panel, where no case is shown, stays on
    // the panel: the stream takes its row away.
    if (!data.shown) return;
    const cases = listed(data.inbox, data.done, data.show, now, data.briefings);
    const at = cases.findIndex((c) => c.id === after.id);
    const next = at >= 0 && at + 1 < cases.length ? page(cases[at + 1]!) : "/";
    navigate(next + query(data.show));
  }

  // The case as serve gave it with a refusal, while it is the one shown
  // and no older than it (withRefused).
  function refused(current: Case): void {
    setData((was) => withRefused(was, current));
    // On the list's page, the refused case stays shown with its refusal, on
    // its own page under the same filter, since the next refresh of the
    // list would show the first case again (IN-04, IN-06).
    if (routeRef.current.page === "list" && home.current && home.current.id === current.id) {
      const show = routeRef.current.show!;
      const path = page(current) + query(show);
      // With the composer open, the entry is the composer's: the case's page
      // takes its place and the composer's goes on again over it, as on a
      // move under the composer (navigate), so that closing the composer,
      // which takes its entry off, lands on the case and not on the list.
      // The entries keep the filter, which back and forward read (back).
      const moved = entry.moved(path);
      history.replaceState({ ...history.state, show }, "", path);
      if (moved) {
        entry.kept();
        history.replaceState({ ...history.state, show }, "");
      }
      // The inbox's query is none, which on a case's page would list the
      // case under its own filter: a stale refusal's case has moved on from
      // the inbox, and the list would move with it. The route keeps the filter.
      const r: Route = { ...where(location)!, show };
      pinned.current = r;
      shownOnce.current = r;
      // A read of / still out would draw its first case over this one.
      loads.current++;
      setRoute(r!);
    }
  }

  const openOptions = () => {
    redraw((n) => n + 1);
    if (optionsDialog.current) optionsDialog.current.showModal();
  };

  const t = tally(data && data.inbox);
  const chip = (on: unknown) => (on ? { class: "on", "aria-current": "page" } : {});
  return html`<header class="top">
  <a class="brand" href="/"><span class="mark" aria-hidden="true"></span>cases</a>
  <span class="rule"></span>
  <${Tally} t=${t} />
  <nav class="chips"><a href="/" ...${chip(data && data.show === "inbox")}>inbox</a><a href="/message" aria-haspopup="dialog">message</a><button type="button" id="options-open" aria-haspopup="dialog" aria-controls="options-dialog" onClick=${openOptions}>options</button></nav>
</header>
<main>
${problem && html`<p class="error notice" role="alert">${problem}</p>`}
<div class="new-build-live" aria-live="polite">${newer && html`<p class="new-build"><span>a new version of cases is running</span><button type="button" onClick=${() => location.reload()}>reload</button></p>`}</div>
${recorded && html`<${Recorded} key=${recorded.at} recorded=${recorded} gone=${() => setRecorded(null)} />`}
${data && html`<${Page} data=${data} now=${now} sent=${sent} refused=${refused} />`}
</main>
<${Options} dialog=${optionsDialog} />
<${Keys} dialog=${keysDialog} />
${composing && data && html`<${Composer} key=${composing.at} to=${composing.to} re=${composing.re} listeners=${data.listeners} cases=${[...data.inbox, ...data.done, ...data.briefings]} now=${now} onSent=${(after: Case) => sent("message", after)} onClose=${((at: number) => () => {
  if (!composingNow.current || composingNow.current.at !== at) return;
  entry.closed();
  shut();
})(composing.at)} back=${composerBack} />`}`;
}

// The box that says what a send recorded. It goes by itself when the
// stylesheet's animation ends, which pointing at it or focusing it holds;
// being in memory only, a reload does not show it again.
function Recorded({ recorded, gone }: { recorded: Sent; gone: () => void }) {
  const end = (e: AnimationEvent) => {
    if (e.animationName === "recorded-out") gone();
  };
  const dismiss = (e: Event) => {
    e.preventDefault();
    gone();
  };
  const said = recorded.event === "message" ? `message sent to ${recorded.to}:` : recorded.event === "amend" ? "edit recorded on" : `${recorded.event} recorded on`;
  return html`<p class="recorded" role="status" onAnimationEnd=${end}><span>${said} <a href=${page(recorded)}>${recorded.title}</a></span><span class="chips dismiss"><a href="#" aria-label="dismiss this confirmation" onClick=${dismiss}>dismiss</a></span></p>`;
}

// The build this page is, as serve wrote it into the page, so that a page
// the browser takes from its cache, as on a move back, knows it is from
// before an upgrade (IN-12); serve's own build when the page lacks it.
async function start(): Promise<void> {
  // The app puts the page back where it was on a move back, once it has
  // drawn it: see the scroll effect in App.
  history.scrollRestoration = "manual";
  const meta = document.querySelector('meta[name="cases-build"]') as HTMLMetaElement | null;
  let build = meta ? meta.content : "";
  if (!build) {
    try {
      build = (await api.index()).version;
    } catch (e) {
      // serve cannot be reached: the page draws what it can, and says so.
    }
  }
  render(html`<${App} build=${build} />`, document.body);
}

start();
