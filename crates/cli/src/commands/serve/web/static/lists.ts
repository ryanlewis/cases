// The list beside a case, under a filter: the inbox (IN-01), the cases with
// an agent, with the messages sent and not yet picked up, and those done
// (IN-07), or all, or the briefings in their own tab (IN-14); the header's
// tally, and the inbox-zero panel, with who is listening for messages
// (IN-13, ST-16).
import { useEffect, useRef, useState } from "preact/hooks";
import * as api from "./api.js";
import { ask, firstLine, groups } from "./ask.js";
import { followUp } from "./followup.js";
import { followed, mark, since as sinceDrawn } from "./fresh.js";
import { html } from "./html.js";
import { Hint } from "./keys.js";
import { page } from "./ids.js";
import { FILTERS, compose, query } from "./routes.js";
import { KindIcon } from "./kinds.js";
import { age, ago, since, thisWeek, today } from "./time.js";
import type { What } from "./api.js";
import type { Filter } from "./routes.js";
import type { ComponentChildren } from "preact";
import type { Mark } from "./fresh.js";
import type { Case, Listener, State, Time } from "./types.js";

// The header's counts.
export interface Tally {
  blocking: number;
  today: number;
  whenever: number;
  parked: number;
  waiting: number;
}

// The inbox-zero panel's figures.
export interface Figures {
  today: number;
  week: number;
  closed: number;
  lastAnswer: string;
  flights: Flight[];
  // The messages the human sent that no agent has picked up, newest first.
  outbox: Case[];
}

// A case with an agent: who has it, and the answer or pickup that put it
// there, at `at`.
export interface Flight {
  c: Case;
  who: string;
  last: string;
  at: Time | undefined;
}

// How many in-flight cases the inbox-zero panel lists; the in flight filter
// has the rest.
export const FLIGHTS = 5;

// How long a case with an agent may go without an event before its row on
// the inbox-zero panel is dimmed as quiet: an hour, as its agent may have
// gone.
export const HUSH = 60 * 60 * 1000;

// How long case `c` has gone without an event, as an age, when that is more
// than HUSH; else "".
export function quiet(c: Case, now: number): string {
  const at = c.updated_at ? Date.parse(c.updated_at) : NaN;
  return !Number.isNaN(at) && now - at > HUSH ? age(at, now) : "";
}

// How long an answered case may wait for an agent to pick its answer up
// before its card and its row on the inbox-zero panel say so: 10 minutes.
// An agent waiting on its case takes the answer within seconds, so one not
// taken by then may have no agent coming for it.
export const UNPICKED = 10 * 60 * 1000;

// How long answered case `c` has waited for an agent to pick its answer up,
// as an age, when that is UNPICKED or more; else "". A case picked up,
// closed or withdrawn has not waited.
export function unpicked(c: Case, now: number): string {
  if (c.state !== "answered") return "";
  const at = c.answer && c.answer.answered_at ? Date.parse(c.answer.answered_at) : NaN;
  return !Number.isNaN(at) && now - at >= UNPICKED ? age(at, now) : "";
}

// How long since the human last answered before the inbox-zero panel says
// when that was: an hour. A newer answer from today is in the day's count
// already; one from before midnight is not, so the panel says when it was.
export const QUIET = 60 * 60 * 1000;

// The states of the cases the human is through with, and of the messages
// the human sent.
export const DONE: State[] = ["answered", "pickedup", "closed", "withdrawn", "sent"];

// Who the page takes to be listening after a read of the listeners gave
// `read`, null when it failed, with `shown` those it took to be listening
// and `missed` the reads in a row that had failed before it: a read that
// worked gives its list; a failed one keeps `shown`, once, as one failed read
// may be a blip; a second in a row makes who listens not known (null), as
// serve drops a listener it has not heard from for 45 s. It gives that, and
// the reads in a row that have failed now.
export function afterRead(read: Listener[] | null, shown: Listener[] | null, missed: number): [Listener[] | null, number] {
  if (read) return [read, 0];
  return [missed === 0 ? shown : null, missed + 1];
}

// How long a listener may go unheard before it is dimmed as perhaps gone:
// 20 s, as it refreshes its record every 15 s. serve lists it no more once it
// has gone 45 s (the store's LAPSE).
export const UNHEARD = 20 * 1000;

// Who a message goes to, for people: its worker, then its labels.
export function addressee(c: Case): string {
  return [c.worker, ...(c.labels || []).map((label) => `label ${label}`)].filter(Boolean).join(" or ");
}

// Whether a wait listening now would take the message `c`: one listens for
// its worker or one of its labels.
export function takes(listeners: Listener[], c: Case): boolean {
  const labels = c.labels || [];
  return listeners.some((l) => l.worker === c.worker || l.waits.some((w) => w.labels.some((label) => labels.includes(label))));
}

// Whether a wait listening now would take case `c`, by the rule `cases
// wait` matches a case by, each wait on its own: the wait's worker is the
// case's, and it names no labels or one of the case's, and no kinds or the
// case's. (A message has its own rule: takes, above.) Only a wait that
// listens is among the listeners: one with --pickup and --worker, no --id,
// and no --kind that leaves out message. So no match says only that the
// page sees none, and a match does not say the wait will take the case:
// without --since, it counts only an answer that landed while it waits.
export function listensForCase(listeners: Listener[], c: Case): boolean {
  const labels = c.labels || [];
  return listeners.some(
    (l) =>
      l.worker === c.worker &&
      l.waits.some((w) => (w.labels.length === 0 || w.labels.some((label) => labels.includes(label))) && (w.kinds.length === 0 || w.kinds.includes(c.kind))),
  );
}

// The flag on answered case `c` whose answer no agent has picked up for
// `long`, from unpicked: for how long, and, where the page knows the
// `listeners` and none of them matches the case, that too, short enough for
// a card's one line, with what it means in its title; nothing more when one
// does, or when who listens is not known (null).
function Unpicked({ c, long, listeners }: { c: Case; long: string; listeners?: Listener[] | null | undefined }) {
  return html`<span class="unpicked">not picked up · ${long}</span>${listeners && !listensForCase(listeners, c) && html` · <span class="hush" title="no listening session's wait matches this case's worker and labels">no listener matches</span>`}`;
}

// How long listener `l` has gone unheard, when that is more than UNHEARD,
// as "not heard from for 30s"; else "".
export function heard(l: Listener, now: number): string {
  const gone = now - Date.parse(l.heard);
  if (!(gone > UNHEARD)) return "";
  const a = age(l.heard, now);
  return `not heard from for ${a === "now" ? `${Math.floor(gone / 1000)}s` : a}`;
}

// The listener of the first of `names` that is listening, if any. The
// listening record is the wait's --worker, and a wait may pick a case up by
// another name, its --by, so the case's worker is looked for after the name
// that has it.
export function listenerOf(listeners: Listener[], ...names: (string | undefined)[]): Listener | undefined {
  for (const name of names) {
    const found = name ? listeners.find((l) => l.worker === name) : undefined;
    if (found) return found;
  }
  return undefined;
}

// What listener `l` says it is doing, with how long ago it said so; nothing
// when it says nothing.
export function Doing({ l, now }: { l: Listener | undefined; now: number }) {
  if (!l || !l.doing) return null;
  return html`<span class="doing">doing: ${l.doing}${l.doing_at ? ` · ${ago(l.doing_at, now)}` : ""}</span>`;
}

// Whether case `c` is with an agent: answered or picked up, not closed.
function inFlight(c: Case): boolean {
  return c.state === "answered" || c.state === "pickedup";
}

// The cases the page read, as the list draws them: `inbox`, the open and
// parked cases; `done`, the others; and `briefings`, the briefings open,
// answered or picked up, which have a tab of their own (IN-14).
export interface Split {
  inbox: Case[];
  done: Case[];
  briefings: Case[];
}

// The cases read as open or parked, `waiting`, and the others, split as the
// list draws them (Split). Briefings are in none of the filters, the tally,
// the title's count or the tab icon's count: they are under their own tab
// until they are closed or withdrawn, so that the human can still withdraw
// an answer an agent has picked up; then they join the done filter's lists,
// so that the history is in one place (IN-14).
export function split(waiting: Case[], others: Case[]): Split {
  const briefing = (c: Case) => c.kind === "briefing";
  return {
    inbox: waiting.filter((c) => !briefing(c)),
    done: others.filter((c) => !briefing(c) || c.state === "closed" || c.state === "withdrawn"),
    briefings: [...waiting.filter(briefing), ...others.filter((c) => briefing(c) && inFlight(c))],
  };
}

// The header's counts: open cases by urgency, and parked cases.
export function tally(inbox: Case[] | null | undefined): Tally {
  const t = { blocking: 0, today: 0, whenever: 0, parked: 0 } as Tally;
  for (const c of inbox || []) {
    if (c.state === "parked") t.parked++;
    else if (c.state === "open" && c.urgency in t) t[c.urgency]++;
  }
  t.waiting = t.blocking + t.today + t.whenever;
  return t;
}

// The tab icon's address for a tally: the open cases, red when one is
// blocking. Parked cases are not counted.
export function icon(t: Tally): string {
  if (!t.waiting) return "/favicon.svg";
  return `/favicon.svg?n=${t.waiting}${t.blocking ? "&blocking=1" : ""}`;
}

// The tally in the header: the counts that are not zero; only blocking is
// highlighted.
export function Tally({ t }: { t: Tally }) {
  const items = ([
    [t.blocking, "blocking", true],
    [t.today, "today", false],
    [t.whenever, "whenever", false],
    [t.parked, "parked", false],
  ] as [number, string, boolean][]).filter(([n]) => n > 0);
  return html`<span id="tally" class="label tally">${items.map(
    ([n, label, hot], i) => html`${i > 0 ? " · " : ""}<span class=${hot ? "hot" : undefined}><strong>${n}</strong> ${label}</span>`,
  )}</span>`;
}

function Labels({ labels }: { labels: string[] | undefined }) {
  return (labels || []).map((label) => html`<span class="tag">${label}</span>`);
}

// A card of the list. An open or parked case says what it is and what it
// asks; one with an agent says who has it and since when, or, answered and
// not picked up for UNPICKED, that it has not been (Unpicked); a message sent and
// not picked up, who it went to, when, and whether anyone listening would
// take it; one closed says its outcome, and one withdrawn why. Its link keeps
// the filter `show`.
function Card({ c, on, show, now, listeners }: { c: Case; on: boolean; show: Filter; now: number; listeners?: Listener[] | null | undefined }) {
  const waiting = c.state === "open" || c.state === "parked";
  const f = waiting ? followUp(c) : null;
  const classes = ["card", waiting ? `urgency-${c.urgency}` : `state-${c.state}`];
  if (c.state === "parked") classes.push("parked");
  if (on) classes.push("selected");
  let line: unknown = waiting ? ask(c, f) : "";
  let tone = c.state === "parked" ? "ask parked" : f ? "ask followup" : "ask";
  if (c.state === "answered" || c.state === "pickedup") {
    const w = flight(c);
    const long = unpicked(c, now);
    line = html`with <strong>${w.who}</strong> · ${long ? html`<${Unpicked} c=${c} long=${long} listeners=${listeners} />` : `${w.last} ${ago(w.at, now)}`}`;
    tone = "ask flight";
  } else if (c.state === "sent") {
    line = html`sent to <strong>${addressee(c)}</strong> · ${ago(c.opened_at, now)}${listeners && !takes(listeners, c) && html` · <span class="hush">nobody is listening</span>`}`;
    tone = "ask flight";
  } else if (c.state === "closed") {
    line = firstLine(c.close && c.close.outcome);
  } else if (c.state === "withdrawn") {
    line = withdrawal(c);
  }
  return html`<a class=${classes.join(" ")} href=${page(c) + query(show)} aria-current=${on ? "page" : undefined}>
  <div class="meta">
    <${KindIcon} kind=${c.kind} />
    ${waiting ? html`<span class="badge urgency">${c.urgency}</span>` : html`<span class="badge state">${STATES[c.state]}</span>`}
    <span>${c.kind}</span>
    ${c.state === "parked" && html`<span class="badge parked">parked</span>`}
    ${c.for && html`<span>for ${c.for}</span>`}
    ${c.worker && html`<span>${c.kind === "message" ? `to ${c.worker}` : c.worker}</span>`}
    <${Labels} labels=${c.labels} />
    <span class="age">${age(waiting ? c.opened_at : c.updated_at, now)}</span>
  </div>
  <h3>${c.title}</h3>
  ${line && html`<p class=${tone}>${line}</p>`}
</a>`;
}

// The states of the cases the human is through with, and of a message the
// human sent, as a card names them.
const STATES: Record<string, string> = { answered: "answered", pickedup: "picked up", closed: "closed", withdrawn: "withdrawn", sent: "sent" };

// Why case `c` was withdrawn, from its last withdraw, if it says.
function withdrawal(c: Case): string {
  const w = (c.events || []).filter((e) => e.event === "withdraw").pop();
  const reason = w && (w.data || {}).reason;
  return typeof reason === "string" ? reason.trim() : "";
}

// The needs-you banner: how many open cases are blocking an agent, who has
// waited on them and for how long, and a link to the first. Any look may
// show it; mono hides it. Nothing when no case is blocking.
export function NeedsYou({ cases, now }: { cases: Case[]; now: number }) {
  const blocking = cases.filter((c) => c.state === "open" && c.urgency === "blocking");
  if (!blocking.length) return null;
  const n = blocking.length;
  const wait = (c: Case) => {
    const a = age(c.opened_at, now);
    return a === "now" ? "under a minute" : a;
  };
  const waits = blocking.map((c, i) => `${c.worker || "an agent"}${i === 0 ? " has waited" : ""} ${wait(c)}`);
  return html`<p class="needs-you"><span><strong>${n} ${n === 1 ? "case" : "cases"}</strong> ${n === 1 ? "is" : "are"} blocking an agent. <small>${waits.join(", ")}.</small></span><a href=${page(blocking[0]!)}>start</a></p>`;
}

// A part of the list under its heading: a group of the inbox, the cases
// with an agent, or those done today or before.
export interface Part {
  key: string;
  label: string;
  cases: Case[];
  // Drawn folded, and left out of the cases the page moves through
  // (listed): the briefings answered, picked up or not.
  folded?: boolean;
  // Said after the count: of the briefings answered, how many no agent
  // has picked up yet.
  note?: string;
}

// The list's parts under the filter `show`. The inbox is open and parked
// cases under Blocking, Today, Whenever and Parked, each in the order serve
// gives them; in flight is the messages the human sent that no agent has
// picked up, newest first, then the cases with an agent, the latest answer
// or pickup first; done is the cases closed or withdrawn, newest first by
// latest event, today's apart from those before (local midnight); all is
// the three in that order. `inbox` is the open and parked cases, and
// `done` the others. The briefings tab lists `briefings`: the open ones,
// those waiting to land before those landed, each in the order serve gives
// them (urgency, then open order), then the answered ones, picked up or
// not, newest answer first, as the TUI orders them, folded, with how many
// no agent has picked up yet (IN-14). Newest first, as the answer just
// given is the one the human is likeliest to look at again or withdraw;
// by the answer, so that a pickup does not move it.
export function parts(inbox: Case[], done: Case[], show: Filter, now: number, briefings: Case[] = []): Part[] {
  const out: Part[] = [];
  if (show === "briefings") {
    const open = briefings.filter((c) => c.state === "open");
    const answered = briefings.filter(inFlight);
    answered.sort((a, b) => newest(a.answer && a.answer.answered_at, b.answer && b.answer.answered_at));
    const unpickedUp = answered.filter((c) => c.state === "answered").length;
    out.push(
      { key: "gated", label: "Waiting to land", cases: open.filter((c) => !c.landed) },
      { key: "landed", label: "Landed", cases: open.filter((c) => c.landed) },
      { key: "answered", label: "Answered", cases: answered, folded: true, ...(unpickedUp ? { note: `${unpickedUp} not picked up` } : {}) },
    );
  }
  if (show === "inbox" || show === "all") {
    out.push(...groups(inbox).map((g) => ({ key: g.group, label: g.label, cases: g.cases })));
  }
  if (show === "inflight" || show === "all") {
    out.push({ key: "sent", label: "Sent", cases: outbox(done) });
    const flights = done.filter(inFlight).map(flight);
    flights.sort((a, b) => newest(a.at, b.at));
    out.push({ key: "inflight", label: "In flight", cases: flights.map((f) => f.c) });
  }
  if (show === "done" || show === "all") {
    const through = done.filter((c) => c.state === "closed" || c.state === "withdrawn");
    through.sort((a, b) => newest(a.updated_at, b.updated_at));
    const midnight = today(now);
    out.push(
      { key: "done-today", label: "Done today", cases: through.filter((c) => since(c.updated_at, midnight)) },
      { key: "done-before", label: "Before today", cases: through.filter((c) => !since(c.updated_at, midnight)) },
    );
  }
  return out.filter((p) => p.cases.length);
}

// The cases under the filter `show`, in the order the list draws them,
// less those in a folded part: the page opens the first, and moves on to the
// next after a send.
export function listed(inbox: Case[], done: Case[], show: Filter, now: number, briefings: Case[] = []): Case[] {
  return parts(inbox, done, show, now, briefings).flatMap((p) => (p.folded ? [] : p.cases));
}

// The messages the human sent that no agent has picked up, newest first.
function outbox(cases: Case[]): Case[] {
  const sent = cases.filter((c) => c.state === "sent");
  sent.sort((a, b) => newest(a.updated_at, b.updated_at));
  return sent;
}

// How many cases each filter lists; for the briefings tab, the open
// briefings (IN-14).
export function counts(inbox: Case[], done: Case[], briefings: Case[] = []): Record<Filter, number> {
  const flying = done.filter((c) => inFlight(c) || c.state === "sent").length;
  const through = done.filter((c) => c.state === "closed" || c.state === "withdrawn").length;
  const reading = briefings.filter((c) => c.state === "open").length;
  return { inbox: inbox.length, inflight: flying, done: through, all: inbox.length + flying + through, briefings: reading };
}

// What an empty filter says.
const EMPTY: Record<Filter, string> = {
  inbox: "nothing is waiting on you.",
  inflight: "nothing is with an agent.",
  done: "nothing is done yet.",
  all: "there are no cases yet.",
  briefings: "no briefings to read.",
};

// The filter row: each filter with its count, the one shown marked. Each
// goes to the list under it, beside its first case. On another filter, the
// inbox stands out while open cases wait on the human (`t`), in the blocking
// colour when one is blocking.
export function Filters({ show, counts, selected, t }: { show: Filter; counts: Record<Filter, number>; selected: string; t: Tally }) {
  const classes = (f: Filter) => {
    const tab = f === "briefings" ? "tab-briefings" : "";
    if (f === show) return `${tab} on`.trim();
    if (f === "inbox" && t.waiting > 0) return t.blocking ? "waiting blocking" : "waiting";
    return tab || undefined;
  };
  return html`<nav class="chips filters" aria-label="filter">${FILTERS.map(
    (f) => html`<a href=${"/" + query(f.show)} class=${classes(f.show)} aria-current=${f.show === show ? (selected ? "true" : "page") : undefined}>${f.label} <span class="count">${counts[f.show]}</span></a>`,
  )}</nav>`;
}

// The waiting notice: on a filter other than the inbox, how many open cases
// wait on the human and how many of them are blocking, with a way to the
// inbox. It stays until the inbox is shown or none wait, and follows the
// store as the list does. Its box is drawn even when empty, as a polite live
// region, so what changes in it is read out without taking the focus. The
// list has one at its top; the page has one to float at the foot of a
// phone's screen (`float`), where the list may be hidden.
export function Waiting({ t, show, float = false }: { t: Tally; show: Filter; float?: boolean }) {
  const on = show !== "inbox" && t.waiting > 0;
  return html`<div class=${float ? "waiting-live float" : "waiting-live"} aria-live="polite">${on && html`<p class=${t.blocking ? "waiting-notice blocking" : "waiting-notice"}><span><strong>${t.waiting}</strong> waiting on you${t.blocking ? html` · <strong class="hot">${t.blocking} blocking</strong>` : ""}</span><a href="/">go to inbox</a></p>`}</div>`;
}

// The keys that move through the list and list the keys, beside the
// filter row (keys.js).
function ListKeys() {
  return html`<p class="list-keys"><${Hint} keys=${["j", "k"]} label="move" /> <${Hint} keys=${["?"]} label="keys" /></p>`;
}

// The list: the filter row, the waiting notice, then the cases under the
// filter `show` in their parts, with `selected` marked, after the needs-you
// banner on the inbox. On another filter the waiting notice says what is
// blocking instead. The cards follow one another in the page as on screen,
// for j and k. With `listeners`, when who listens is known, a sent
// message's card says when no one listening would take it, and an answer
// not picked up when no listener matches it.
export function List({
  inbox,
  done,
  show,
  selected,
  now,
  listeners,
  briefings = [],
}: {
  inbox: Case[];
  done: Case[];
  show: Filter;
  selected: string;
  now: number;
  listeners?: Listener[] | null | undefined;
  briefings?: Case[];
}) {
  const ps = parts(inbox, done, show, now, briefings);
  const t = tally(inbox);
  const cards = (p: Part) => p.cases.map((c) => html`<${Card} key=${c.id} c=${c} on=${c.id === selected} show=${show} now=${now} listeners=${listeners} />`);
  const part = (p: Part) =>
    p.folded
      ? html`<${Folded} key=${p.key} p=${p} holds=${p.cases.some((c) => c.id === selected)}>${cards(p)}<//>`
      : html`<section key=${p.key} class=${`group group-${p.key}`} aria-labelledby=${`group-${p.key}`}>
  <h2 id=${`group-${p.key}`} class="label">${p.label} <span class="count">${p.cases.length}</span>${note(p)}</h2>
  ${cards(p)}
</section>`;
  // The briefings tab says it has none to read above a folded part.
  const empty = !ps.some((p) => !p.folded) && html`<p class="empty">${EMPTY[show]}</p>`;
  return html`<div class="filter-row"><${Filters} show=${show} counts=${counts(inbox, done, briefings)} selected=${selected} t=${t} /><${ListKeys} /></div><${Waiting} t=${t} show=${show} /><div id="inbox">${show === "inbox" && html`<${NeedsYou} cases=${inbox} now=${now} />`}${empty}${ps.map(part)}</div>`;
}

// What a part's heading says after its count, if anything.
function note(p: Part) {
  return p.note ? html` <span class="note">· ${p.note}</span>` : null;
}

// A folded part of the list: it opens itself when it comes to hold the case
// shown, and otherwise stays as the human left it.
function Folded({ p, holds, children }: { p: Part; holds: boolean; children?: ComponentChildren }) {
  const [open, setOpen] = useState(holds);
  useEffect(() => {
    if (holds) setOpen(true);
  }, [holds]);
  return html`<details class=${`group group-${p.key}`} open=${open || undefined} onToggle=${(e: Event) => setOpen((e.target as HTMLDetailsElement).open)}>
  <summary class="label">${p.label} <span class="count">${p.cases.length}</span>${note(p)}</summary>
  ${children}
</details>`;
}

// Case `c`, which is with an agent, as a Flight.
function flight(c: Case): Flight {
  let who = c.worker;
  let last = "answered";
  let at = c.answer && c.answer.answered_at;
  if (c.state === "pickedup" && c.pickup) {
    last = "picked up";
    at = c.pickup.picked_up_at;
    if (c.pickup.by) who = c.pickup.by;
  }
  return { c, who: who || "an agent", last, at };
}

// Newest first. Times compare as times: RFC 3339 text with a fraction does
// not sort as its time.
function newest(a: Time | undefined, b: Time | undefined): number {
  const time = (at: Time | undefined) => (at ? Date.parse(at) : 0);
  return time(b) - time(a);
}

// Whether case `c` was closed at or after `from`.
function closedSince(c: Case, from: number): boolean {
  return c.state === "closed" && (c.events || []).some((e) => e.event === "close" && since(e.at, from));
}

// What was got through, for the inbox-zero panel: the cases the human
// answered, parked or resumed today and this week, those closed today,
// those still with an agent, the answers not picked up for UNPICKED first,
// as the panel lists only FLIGHTS of them, then the latest answer or pickup
// first, and when the human last answered, if that was more than QUIET ago
// or before today.
export function zero(cases: Case[], now: number): Figures {
  const day = today(now);
  const week = thisWeek(now);
  const z: Figures = { today: 0, week: 0, closed: 0, lastAnswer: "", flights: [], outbox: outbox(cases) };
  let last = 0;
  for (const c of cases) {
    if (inFlight(c)) z.flights.push(flight(c));
    if (closedSince(c, day)) z.closed++;
    let onDay = false;
    let inWeek = false;
    for (const e of c.events || []) {
      if (e.author !== "human" || !["answer", "park", "resume"].includes(e.event)) continue;
      onDay = onDay || since(e.at, day);
      inWeek = inWeek || since(e.at, week);
      if (e.event === "answer" && e.at && new Date(e.at).getTime() > last) last = new Date(e.at).getTime();
    }
    if (onDay) z.today++;
    if (inWeek) z.week++;
  }
  if (last && (now - last > QUIET || last < day)) z.lastAnswer = age(last, now);
  const flagged = (w: Flight) => (unpicked(w.c, now) ? 0 : 1);
  z.flights.sort((a, b) => flagged(a) - flagged(b) || newest(a.at, b.at));
  return z;
}

// What the page does with a write made from the panel: as with the case
// view's (app.ts), it says so once.
type OnSent = (what: What, after: Case) => void;

// The messages the human sent that no agent has picked up, on the
// inbox-zero panel under their own heading, first, as the one thing there
// that may need the human: the first FLIGHTS of them, each linking to its
// message, with who it went to, when, and whether anyone listening would
// take it. The rest are a link to the in flight filter.
function Outbox({ outbox, listeners, now }: { outbox: Case[]; listeners?: Listener[] | null | undefined; now: number }) {
  if (!outbox.length) return null;
  const more = outbox.length - FLIGHTS;
  return html`<section class="outbox" aria-labelledby="outbox-head"><h2 class="section" id="outbox-head">sent, not picked up</h2><ul class="flights">${outbox.slice(0, FLIGHTS).map((c) => {
    const nobody = !!listeners && !takes(listeners, c);
    return html`<li key=${c.id} class=${nobody ? "quiet" : undefined}><a href=${page(c)}>${c.title}</a><span class="flight">to <strong>${addressee(c)}</strong> · sent ${ago(c.opened_at, now)}${nobody && html` · <span class="hush">nobody is listening for ${addressee(c)}</span>`}</span></li>`;
  })}${more > 0 && html`<li class="more"><a href="/?show=inflight">${more} more sent</a></li>`}</ul></section>`;
}

// The in-flight cases on the inbox-zero panel, under their own heading,
// the first FLIGHTS of them: each links to its case and says who has it and
// since when, as its card does, or that its answer has not been picked up,
// or, when it has gone quiet, for how long, dimmed, and what its worker says
// it is doing when it is listening; and
// each can be withdrawn. The rest are a link to the in flight filter.
function Flights({ flights, listeners, now, sent }: { flights: Flight[]; listeners?: Listener[] | null | undefined; now: number; sent?: OnSent | undefined }) {
  if (!flights.length) return null;
  const more = flights.length - FLIGHTS;
  return html`<section class="in-flight" aria-labelledby="in-flight-head"><h2 class="section" id="in-flight-head">still with agents</h2><ul class="flights">${flights.slice(0, FLIGHTS).map((w) => {
    // An answer not picked up says so, over quiet: it is the one to act on.
    const flagged = unpicked(w.c, now);
    const hushed = flagged ? "" : quiet(w.c, now);
    return html`<li key=${w.c.id} class=${hushed ? "quiet" : undefined}><a href=${page(w.c)}>${w.c.title}</a><span class="flight">with <strong>${w.who}</strong> · ${flagged ? html`<${Unpicked} c=${w.c} long=${flagged} listeners=${listeners} />` : hushed ? html`${w.last} · <span class="hush">quiet ${hushed}</span>` : `${w.last} ${ago(w.at, now)}`}</span><${Doing} l=${listeners ? listenerOf(listeners, w.who, w.c.worker) : undefined} now=${now} /><${RowWithdraw} c=${w.c} sent=${sent} /></li>`;
  })}${more > 0 && html`<li class="more"><a href="/?show=inflight">${more} more in flight</a></li>`}</ul></section>`;
}

// How long listener `l` has listened, for people: "since just now", or
// "for 2h".
function listened(l: Listener, now: number): string {
  const a = age(l.since, now);
  return a === "now" ? "listening since just now" : `listening for ${a}`;
}

// The workers listening for messages now, on the inbox-zero panel under
// their own heading, the first FLIGHTS of them: each with its labels, how
// long it has listened, what it says it is doing, and a link to message it;
// dimmed, with how long, when it has missed a heartbeat and may have gone.
// The rest are a link to the composer, whose picker lists them all.
function Listening({ listeners, now }: { listeners?: Listener[] | null | undefined; now: number }) {
  if (!listeners || !listeners.length) return null;
  const more = listeners.length - FLIGHTS;
  return html`<section class="listening" aria-labelledby="listening-head"><h2 class="section" id="listening-head">listening now</h2><ul class="flights">${listeners.slice(0, FLIGHTS).map((l) => {
    const hush = heard(l, now);
    return html`<li key=${l.worker} class=${hush ? "quiet" : undefined}><strong>${l.worker}</strong>${l.labels.map((label) => html` <span class="tag">${label}</span>`)}<span class="flight">${listened(l, now)}${hush && html` · <span class="hush">${hush}</span>`}</span><${Doing} l=${l} now=${now} /><a class="row-message" href=${compose(l.worker)} aria-label=${`message ${l.worker}`}>message</a></li>`;
  })}${more > 0 && html`<li class="more"><a href="/message">${more} more listening</a></li>`}</ul></section>`;
}

// An open withdraw line: the state the human last saw its case in, and the
// mark its withdraw is sent at. The mark is that state's only when the case
// just moved on to it; after any other change it is older, so that the
// withdraw is refused as stale.
export interface Line {
  state: State;
  drawn: Mark;
}

// The line drawn from case `c` as the human is shown it: on pressing
// withdraw, and on a refusal, which shows the case as it is now.
export function lineFrom(c: Case): Line {
  return { state: c.state, drawn: mark(c) };
}

// What an open line becomes once the page's stream brings case `c`, with
// what it says, if anything. A case at or below the line's mark is one the
// line has already taken, such as from a refusal, that the stream brings
// late: it changes nothing. In the state it was drawn in, it follows the
// case as the case view's forms do (`followed`, fresh.ts). In another state
// that the case only moved on to (`since`, fresh.ts), such as by a pickup, it
// takes the case as it is now and says the state, so that the withdraw is
// never sent for a state the human has not seen. After anything else, such
// as a reopen and a new answer between two reads, it keeps its mark, so that
// the withdraw is refused as stale and the refusal says what changed (IN-05,
// IN-06).
export function lineFollows(was: Line, c: Case): { line: Line; said?: string } {
  if (c.revision <= was.drawn.revision) return { line: was };
  if (c.state === was.state) return { line: { state: was.state, drawn: followed(was.drawn, c) } };
  const now = STATES[c.state] || c.state;
  if (sinceDrawn(c, was.drawn) === "moved") {
    return { line: lineFrom(c), said: `this case was ${now} since you pressed withdraw. withdraw it if you still mean to.` };
  }
  return { line: { state: c.state, drawn: was.drawn }, said: `this case changed since you pressed withdraw, and is ${now} now.` };
}

// The human's withdraw of a case still with an agent, from its row (ST-14):
// a button, then, inline, an optional reason and a second button that
// withdraws at the revision the row had when the first was pressed, so one
// stray click does nothing. A refusal is said in the row, and a stale one
// takes the case as it is now, to be sent again once seen. While the line is
// open it follows what the page's stream brings as `lineFollows` says. The
// stream takes the row away once the withdraw lands. Escape or keep puts the
// button back, with the focus on it.
function RowWithdraw({ c, sent }: { c: Case; sent?: OnSent | undefined }) {
  const [open, setOpen] = useState(false);
  const line = useRef<Line>(lineFrom(c));
  const [reason, setReason] = useState("");
  const [refusal, setRefusal] = useState("");
  // The send that is out, which only that send lets go (IN-09).
  const sending = useRef<object | null>(null);
  // The send that went last: only its answer speaks for the line.
  const newest = useRef<object | null>(null);
  const field = useRef<HTMLInputElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const shut = useRef(false);
  useEffect(() => {
    if (open && field.current) field.current.focus();
    if (!open && shut.current && button.current) button.current.focus();
  }, [open]);
  useEffect(() => {
    if (!open) return;
    const { line: next, said } = lineFollows(line.current, c);
    line.current = next;
    if (said) setRefusal(said);
  }, [c.revision, c.state]);
  function start(): void {
    line.current = lineFrom(c);
    setRefusal("");
    setOpen(true);
  }
  function keep(): void {
    shut.current = true;
    setOpen(false);
    setReason("");
    setRefusal("");
  }
  async function submit(e: Event): Promise<void> {
    e.preventDefault();
    if (sending.current) return;
    const self = {};
    sending.current = self;
    newest.current = self;
    // As the case view's (sendOnce, case.ts): a send while one is out does
    // nothing, until it is answered or 10 s have passed (IN-09). One slower
    // than 10 s does not, when it comes back, let go of the send after it.
    const done = () => {
      if (sending.current === self) sending.current = null;
    };
    const release = setTimeout(done, 10000);
    const why = reason.trim();
    try {
      const { revision } = line.current.drawn;
      const { after, refusal: problem, current } = await api.send(c.id, "withdraw", why ? { revision, reason: why } : { revision });
      if (after) {
        if (sent) sent("withdraw", after);
        return;
      }
      // A send another went after, as this one was out for over 10 s, says
      // nothing: the newer send's answer does.
      if (newest.current !== self) return;
      setRefusal(problem);
      // The case as the refusal shows it, unless the stream has already
      // brought a later one: that the line follows as it would have.
      if (current) {
        const { line: next, said } = lineFollows(lineFrom(current), c);
        line.current = next;
        if (said) setRefusal(said);
      }
    } finally {
      clearTimeout(release);
      done();
    }
  }
  if (!open) {
    return html`<button type="button" class="row-withdraw" ref=${button} aria-label=${`withdraw ${c.title}`} onClick=${start}>withdraw</button>`;
  }
  return html`<form class="row-confirm" data-write="withdraw" onSubmit=${submit} onKeyDown=${(e: KeyboardEvent) => e.key === "Escape" && keep()}>
  <label>reason, if any <input type="text" ref=${field} name="reason" value=${reason} onInput=${(e: Event) => setReason((e.target as HTMLInputElement).value)} /></label>
  <div class="actions"><button type="submit">withdraw it</button><button type="button" onClick=${keep}>keep</button></div>
  ${refusal && html`<p class="error" role="alert">${refusal}</p>`}
</form>`;
}

// The inbox-zero panel: one line of who did what, with the parts that are
// not zero, the messages sent and not picked up, the cases still with
// agents, who is listening for messages, and when the human last answered,
// if that was a while ago; and a link to send a message. Without
// `listeners`, as with them null, who listens is not known, as on List.
export function Zero({ z, now, sent, listeners }: { z: Figures; now: number; sent?: OnSent; listeners?: Listener[] | null | undefined }) {
  let you = null;
  if (z.today) {
    you = html`you answered <strong>${z.today}</strong> today${z.week > z.today ? html` (<strong>${z.week}</strong> this week)` : ""}`;
  } else if (z.week) {
    you = html`you answered <strong>${z.week}</strong> this week`;
  }
  // Closed counts today's only, so it says so beside a week's figure.
  const agents = z.closed > 0 && html`agents <a href="/?show=done">closed <strong>${z.closed}</strong></a>${z.week && !z.today ? " today" : ""}`;
  const nothing = !(you || agents || z.flights.length || z.outbox.length || (listeners && listeners.length) || z.lastAnswer);
  return html`<div class="zero-panel">
  <h1>nothing needs you.</h1>
  ${(you || agents) && html`<p class="who-did">${you}${you && agents && " · "}${agents}</p>`}
  <${Outbox} outbox=${z.outbox} listeners=${listeners} now=${now} />
  <${Flights} flights=${z.flights} listeners=${listeners} now=${now} sent=${sent} />
  <${Listening} listeners=${listeners} now=${now} />
  ${z.lastAnswer && html`<p class="age">last answer ${z.lastAnswer === "now" ? "just now" : `${z.lastAnswer} ago`}</p>`}
  ${nothing && html`<p>nothing has needed you yet.</p>`}
  <p class="send-message"><a href="/message">send a message</a></p>
</div>`;
}
