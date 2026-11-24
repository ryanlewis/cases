// The page for what was read (IN-01, IN-07, IN-13): the list under its
// filter beside the case shown, or beside the inbox-zero panel when the
// inbox is empty. The composer of a message opens over it (app.ts).
import { CaseView } from "./case.js";
import { html } from "./html.js";
import { List, Waiting, Zero, tally, zero } from "./lists.js";
import { FILTERS } from "./routes.js";
import type { What } from "./api.js";
import type { Filter, Route } from "./routes.js";
import type { Case, Listener } from "./types.js";

// The case a page shows: the case, or that there is none with its id.
export interface Shown {
  id: string;
  case?: Case;
  missing?: boolean;
}

// What the page shows, for the route it was read for: see App.
export interface Data {
  route: Route;
  show: Filter;
  inbox: Case[];
  done: Case[];
  // The briefings open, answered or picked up, in their own tab (split,
  // IN-14).
  briefings: Case[];
  shown: Shown | null;
  // The workers listening for messages now; null while who listens is not
  // known: before a read of them has worked, and after two in a row have
  // failed (afterRead).
  listeners: Listener[] | null;
}

// What page `was` shows once a send of the case shown is refused with the
// case as serve gave it, `current`: that case in its place, while it is the
// one shown and no older than it. A refusal that comes back after the page
// moved on, or after a later write showed a newer case, leaves the page as
// it is.
export function withRefused(was: Data | null, current: Case): Data | null {
  return was && was.shown && was.shown.case && was.shown.case.id === current.id && current.revision >= was.shown.case.revision ? { ...was, shown: { ...was.shown, case: current } } : was;
}

// The list under its filter beside its first case or the case chosen. The
// inbox filter with nothing in it has the inbox-zero panel beside it, in the
// same split, so the first case to arrive takes the panel's place and the
// list stays where it is. Another filter with nothing in it leaves the pane
// empty: its list says so. On a phone the waiting notice floats at the foot
// of the screen, over the list or the case.
export function Page({
  data,
  now,
  sent,
  refused,
}: {
  data: Data;
  now: number;
  sent: (what: What, after: Case) => void;
  refused: (current: Case) => void;
}) {
  const { route, show, inbox, done, briefings = [], shown, listeners } = data;
  const c = shown && shown.case;
  const label = FILTERS.find((f) => f.show === show)!.label;
  const list = html`<aside class="list" aria-label=${label}><${List} inbox=${inbox} done=${done} briefings=${briefings} show=${show} selected=${c ? c.id : ""} now=${now} listeners=${listeners} /></aside>`;
  const empty = route.page === "list" && !shown && show === "inbox";
  const all = [...inbox, ...done, ...briefings];
  let work = null;
  if (c) {
    work = html`<${CaseView} key=${c.id} c=${c} now=${now} cases=${all} listeners=${listeners || []} onSent=${sent} onCase=${refused} />`;
  } else if (shown) {
    work = html`<p class="empty">no case ${shown.id} in this store.</p>`;
  } else if (empty) {
    // The panel is the inbox's: briefings until they close are in their own
    // tab, and the panel counts none of them (IN-14).
    work = html`<${Zero} z=${zero([...inbox, ...done], now)} now=${now} sent=${sent} listeners=${listeners} />`;
  }
  const at = route.page === "list" ? (empty ? "split home zero" : "split home") : "split";
  return html`<div class=${at}>${list}<div class="work">${work}</div><${Waiting} t=${tally(inbox)} show=${show} float /></div>`;
}
