// The composer (IN-13): the human writes a note to an agent, in a dialog
// over the page, and picks who takes it: a worker, or a label, whose first
// listening session takes it. Opened for someone, it says who and leaves
// the picker away until asked. The picker puts the agents listening now
// first, from the listening record, then the workers and labels of recent
// cases, which take a message once they listen again. Those not listening
// are folded behind one line, with a filter, as there can be many. A label
// someone listens on is sent to as "another". The same listening rows are
// the inbox-zero panel's (lists.ts).
import type { RefObject } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import * as api from "./api.js";
import { Text } from "./case.js";
import { html } from "./html.js";
import { Hint, SEND, SEND_SHORTCUTS } from "./keys.js";
import { Doing, heard } from "./lists.js";
import { page, pageOf } from "./ids.js";
import { ago } from "./time.js";
import type { Case, Listener } from "./types.js";

// Who a message goes to: a worker by its name, or a label.
export interface Target {
  worker?: string;
  label?: string;
}

// A choice of the picker: its target, the listener it is when it is an
// agent listening now, and whether a session listening now would take it.
export interface Pick {
  key: string;
  target: Target;
  listener?: Listener;
  live: boolean;
}

// How far back a case's worker or label is offered when it is not
// listening: two days.
export const RECENT = 2 * 24 * 60 * 60 * 1000;

// The picker's choices: the agents listening now, then the workers and
// labels of the cases of the last RECENT that are not listening, each once,
// in name order within each part. A label someone listens on is not
// offered: not as listening, where only agents are, nor as not listening.
// A message itself is not a case of the worker it went to, so it adds no
// worker or label.
export function picks(listeners: Listener[], cases: Case[], now: number): Pick[] {
  const byName = (a: Pick, b: Pick) => (a.key < b.key ? -1 : a.key > b.key ? 1 : 0);
  const workers = new Map<string, Pick>();
  const labels = new Set<string>();
  for (const l of listeners) {
    workers.set(l.worker, { key: `worker:${l.worker}`, target: { worker: l.worker }, listener: l, live: true });
    for (const label of l.labels) labels.add(label);
  }
  const idle = new Map<string, Pick>();
  for (const c of cases) {
    if (c.kind === "message") continue;
    const at = Date.parse(c.updated_at || c.opened_at || "");
    if (Number.isNaN(at) || now - at > RECENT) continue;
    if (c.worker && !workers.has(c.worker) && !idle.has(`worker:${c.worker}`)) {
      idle.set(`worker:${c.worker}`, { key: `worker:${c.worker}`, target: { worker: c.worker }, live: false });
    }
    for (const label of c.labels || []) {
      if (!labels.has(label) && !idle.has(`label:${label}`)) {
        idle.set(`label:${label}`, { key: `label:${label}`, target: { label }, live: false });
      }
    }
  }
  const quiet = [...idle.values()];
  return [
    ...[...workers.values()].sort(byName),
    ...quiet.filter((p) => p.target.worker).sort(byName),
    ...quiet.filter((p) => p.target.label).sort(byName),
  ];
}

// What the composer's fields hold.
interface Fields {
  pick: string;
  other: string;
  otherKind: "worker" | "label";
  body: string;
}

// The longest a title taken from a note is, in characters, before the
// ellipsis a cut adds.
const TITLE_MAX = 80;

// `line` without the markdown a title has no use for: the markers at its
// start (headings, quotes, list items), then emphasis stars and backticks
// anywhere, with its runs of spaces made one. A code fence's line is none
// of it: its info string (`sh`) is no title.
function unmarked(line: string): string {
  let rest = line.trim();
  for (let before = ""; rest !== before; ) {
    before = rest;
    rest = rest.replace(/^(#+(?=\s|$)|>|[-*+](?=\s)|\d+[.)](?=\s))\s*/, "");
  }
  if (/^(```|~~~)/.test(rest)) return "";
  return rest.replace(/[*`]/g, "").split(/\s+/).filter(Boolean).join(" ");
}

// The title of a message, which the human writes as a note alone: the
// note's first line with a letter or a digit once its markdown is stripped,
// cut at a word boundary at TITLE_MAX characters with an ellipsis when cut.
// "" when no line has one. `cases message` takes a title with no --title
// the same way (title_of in message.rs), and the tests of both use the same
// examples.
export function title(note: string): string {
  const line = note.split("\n").map(unmarked).find((l) => /[\p{Alphabetic}\p{N}]/u.test(l));
  if (line === undefined) return "";
  const chars = [...line];
  if (chars.length <= TITLE_MAX) return line;
  const head = chars.slice(0, TITLE_MAX).join("");
  const at = head.lastIndexOf(" ");
  const cut = chars[TITLE_MAX] === " " || at <= 0 ? head : head.slice(0, at);
  return `${cut.trimEnd()}…`;
}

// The message the fields give, or the problem that keeps it from being
// sent. Text is trimmed, as a note is, and the title is the note's first
// line.
export function drafted(f: Fields, choices: Pick[], re: string): { body: api.NewMessage; problem?: never } | { body?: never; problem: string } {
  let target: Target | undefined;
  if (f.pick === "other") {
    const name = f.other.trim();
    if (!name) return { problem: "name the worker or label it goes to" };
    target = f.otherKind === "worker" ? { worker: name } : { label: name };
  } else {
    target = choices.find((p) => p.key === f.pick)?.target;
    if (!target) return { problem: "choose who it goes to" };
  }
  const text = f.body.trim();
  const heading = title(text);
  if (!heading) return { problem: "write a note" };
  // How soon is not asked: a message is today's.
  const body: api.NewMessage = { kind: "message", urgency: "today", title: heading, body: text };
  if (target.worker) body.worker = target.worker;
  if (target.label) body.labels = [target.label];
  if (re) body.re = re;
  return { body };
}

// The choices, with the one picked kept when it is offered no more, such as
// a worker that stopped listening while the message was written: the
// message goes to it still, and waits until it listens.
export function kept(choices: Pick[], key: string): Pick[] {
  if (!key || key === "other" || choices.some((p) => p.key === key)) return choices;
  const name = key.slice(key.indexOf(":") + 1);
  const target: Target = key.startsWith("worker:") ? { worker: name } : { label: name };
  return [...choices, { key, target, live: false }];
}

// The composer's keys: Cmd or Ctrl+Enter sends it, once however long they
// are held (IN-09). Enter alone in one of its fields does not, though a
// browser submits a form from a text field or a radio: an agent takes a
// message within a second or two, too soon to edit one sent before its body
// was written.
export function onKey(e: KeyboardEvent): void {
  if (e.key !== "Enter" || e.isComposing) return;
  if (e.metaKey || e.ctrlKey) {
    e.preventDefault();
    if (!e.repeat) (e.currentTarget as HTMLFormElement).requestSubmit();
  } else if ((e.target as HTMLElement).tagName === "INPUT") {
    e.preventDefault();
  }
}

// The choices not listening that the filter `text` leaves: those whose
// worker or label has it in its name, whatever its case, and the one
// picked, `key`, always, so that a choice made is never hidden.
export function filtered(choices: Pick[], text: string, key: string): Pick[] {
  const want = text.trim().toLowerCase();
  if (!want) return choices;
  return choices.filter((p) => p.key === key || (p.target.worker || p.target.label || "").toLowerCase().includes(want));
}

// The dot between the parts of a choice, kept on the line of the part
// before it, so that a narrow picker never starts a line with it.
const DOT = "\u00a0· ";

// One choice of the picker: its name and, for an agent listening now, since
// when and what it says it is doing.
function Choice({ p, checked, onPick, now }: { p: Pick; checked: boolean; onPick: (key: string) => void; now: number }) {
  const l = p.listener;
  const name = p.target.worker ? html`<strong>${p.target.worker}</strong>` : html`label <span class="tag">${p.target.label}</span>`;
  let about: unknown = "";
  if (l) {
    const hush = heard(l, now);
    about = html`${DOT}listening since ${ago(l.since, now)}${hush && html`${DOT}<span class="hush">${hush}</span>`}<${Doing} l=${l} now=${now} />`;
  }
  return html`<label class=${l && heard(l, now) ? "choice pick quiet" : "choice pick"}><input type="radio" name="to" value=${p.key} checked=${checked} onClick=${() => onPick(p.key)} onChange=${() => onPick(p.key)} /> <span>${name}${about}</span></label>`;
}

// The pick of a composer opened for the worker `to`, of the choices
// `offered`, and the name typed for another: `to`'s own choice; else, when
// `about`, the case a reply is about, is `to`'s, its worker, even when that
// case is older than RECENT, as kept() offers it then; else another by that
// name. Nothing is picked for no one.
export function opening(to: string, about: Case | undefined, offered: Pick[]): { pick: string; other: string } {
  if (!to) return { pick: "", other: "" };
  const known = offered.find((p) => p.target.worker === to)?.key || (about?.worker === to ? `worker:${to}` : undefined);
  return known ? { pick: known, other: "" } : { pick: "other", other: to };
}

// Who a message goes to, for people, from the pick `key` and the name typed
// for another: "worker", or "label x".
function named(key: string, f: Fields): string {
  if (key === "other") return f.otherKind === "label" ? `label ${f.other.trim()}` : f.other.trim();
  const name = key.slice(key.indexOf(":") + 1);
  return key.startsWith("label:") ? `label ${name}` : name;
}

// The history as the composer's entry uses it: `window.history`, or a stub
// in a test.
export interface Places {
  pushState(data: unknown, unused: string, url?: string | null): void;
  replaceState(data: unknown, unused: string, url?: string | null): void;
  back(): void;
}

// The entry an open composer puts in the history, at the address of the
// page under it, so that the back button, or a phone's back gesture,
// closes the composer rather than leaving the page under it. The app keeps
// one (app.ts), and tells it what happens:
//
// - opened: the entry goes on the history, once, holding `what` the
//   composer was opened for. One opened while the back a close made is
//   still to land waits for it, so that the back does not take the new
//   entry off.
// - adopted: the page moved onto an entry of a composer closed before, as
//   forward does; it is this composer's entry again.
// - popped: a move in the history (popstate). Whether it took the entry
//   off, so that back is the composer's to answer, as Escape is. The one a
//   close makes is not.
// - kept: back asked about a written note, so the entry goes back on at
//   once: until the human answers, a second back asks again rather than
//   moving the page under the composer; discard takes it off (closed).
// - closed: the composer closed some other way (Escape, send, a click
//   beside it, the close button), and its entry, if still on, comes off
//   with history.back(), which leaves the history as it was before.
// - moved: the app goes to `path` while the entry is on: the page takes
//   the entry's place, with replaceState, so that it is not a back that
//   lands after the move.
export class Entry {
  #on = false;
  #what: unknown = true;
  // A back that `closed` made and the history has not answered yet, and
  // whether the composer was opened again meanwhile.
  #backing = false;
  #wanted = false;
  #places: Places;
  constructor(places: Places) {
    this.#places = places;
  }

  get on(): boolean {
    return this.#on;
  }

  opened(what: unknown = this.#what): void {
    this.#what = what;
    if (this.#on) return;
    if (this.#backing) {
      this.#wanted = true;
      return;
    }
    this.#places.pushState({ composer: what }, "");
    this.#on = true;
  }

  adopted(what: unknown): void {
    this.#what = what;
    this.#on = true;
  }

  popped(): boolean {
    if (this.#backing) {
      this.#backing = false;
      if (this.#wanted) {
        this.#wanted = false;
        this.opened();
      }
      return false;
    }
    const was = this.#on;
    this.#on = false;
    return was;
  }

  kept(): void {
    this.opened();
  }

  closed(): void {
    this.#wanted = false;
    if (!this.#on) return;
    this.#on = false;
    this.#backing = true;
    this.#places.back();
  }

  moved(path: string): boolean {
    if (!this.#on) return false;
    this.#on = false;
    this.#places.replaceState(null, "", path);
    return true;
  }
}

// What the composer's ways out read and change: its fields, whether it asks
// before it closes, and its dialog.
interface Leaving {
  f: Fields;
  leaving: boolean;
  setLeaving: (leaving: boolean) => void;
  dialog: RefObject<HTMLDialogElement>;
}

// Closes the dialog, or, with a note written, asks first.
function leave(c: Leaving): void {
  if (c.f.body.trim() && !c.leaving) {
    c.setLeaving(true);
    return;
  }
  c.dialog.current?.close();
}

// Back does what Escape does: closes the composer, or asks first when a
// note is written. Whether it asked.
function backed(c: Leaving): boolean {
  if (c.f.body.trim()) {
    c.setLeaving(true);
    return true;
  }
  c.dialog.current?.close();
  return false;
}

// The case a reply is about is shown on the page behind the dialog: the
// app goes to it, and the composer goes, once the human says so when a
// note is written. One opened in another tab leaves the dialog as it is.
function toCase(c: Leaving, e: MouseEvent): void {
  if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
  if (c.f.body.trim()) {
    e.preventDefault();
    c.setLeaving(true);
  }
}

// The guard of the send that went last: held until it is answered, or for
// ten seconds (IN-09), and for good once a message is sent.
interface Guard {
  held: boolean;
  // Whether its send was taken: a message sent.
  sent?: boolean;
}

// What a send reads and changes besides: the choices, the case it is about,
// the guard of the send that went last, and where it says what came of it.
interface Sending {
  f: Fields;
  choices: Pick[];
  re: string;
  sending: { current: Guard | null };
  dialog: RefObject<HTMLDialogElement>;
  setRefusal: (refusal: string) => void;
  onSent: (after: Case) => void;
}

// Sends the message the fields give, once, and closes the composer when it
// is sent; else says why not. Each send has a guard of its own, and lets go
// of only that one: a send slower than ten seconds does not, when it comes
// back, let go of the send that went after it. A message sent counts, from
// whichever send it came: the composer goes, and its guard holds for good,
// so the answer of a send after it changes nothing; a refusal counts only
// from the send that went last.
async function submit(s: Sending, e: Event): Promise<void> {
  e.preventDefault();
  if (s.sending.current?.held) return;
  const { body, problem } = drafted(s.f, s.choices, s.re);
  if (problem) {
    s.setRefusal(problem);
    return;
  }
  const guard: Guard = { held: true };
  s.sending.current = guard;
  const release = setTimeout(() => (guard.held = false), 10000);
  try {
    const { after, refusal: why } = await api.sendMessage(body!);
    if (after && !s.sending.current?.sent) s.sending.current = guard;
    if (s.sending.current !== guard) return;
    if (after) {
      // Held for good, though ten seconds have let it go.
      guard.sent = guard.held = true;
      s.setRefusal("");
      s.dialog.current?.close();
      s.onSent(after);
      return;
    }
    s.setRefusal(why);
  } finally {
    clearTimeout(release);
    // A message sent is not sent again: the composer goes once it is
    // sent, and a second send before then, which no revision refuses,
    // would be a second message.
    if (!guard.sent) guard.held = false;
  }
}

// The dialog's own events: Escape, and a click on the dialog itself, which
// is on the backdrop: the composer fills it, on a phone too (style.css). A
// press that starts inside and ends outside, as a drag does, is not.
function dialogEvents(c: Leaving, down: { current: boolean }) {
  return {
    onCancel: (e: Event) => {
      e.preventDefault();
      leave(c);
    },
    onPointerDown: (e: PointerEvent) => (down.current = e.target === e.currentTarget),
    onClick: (e: MouseEvent) => {
      if (e.target === e.currentTarget && down.current) leave(c);
      down.current = false;
    },
  };
}

// Opens as a modal, with the note focused.
function useShown(dialog: RefObject<HTMLDialogElement>): void {
  useEffect(() => {
    const d = dialog.current;
    if (!d) return;
    if (!d.open) d.showModal();
    d.querySelector<HTMLTextAreaElement>('textarea[name="body"]')?.focus();
  }, []);
}

// The picker, once asked for, takes the focus: its choice, or its first.
function usePickerFocus(picker: RefObject<HTMLFieldSetElement>, picking: boolean, to: string): void {
  useEffect(() => {
    if (!picking || !to) return;
    const radios = picker.current?.querySelectorAll<HTMLInputElement>('input[name="to"]');
    const radio = radios && (Array.from(radios).find((r) => r.checked) || radios[0]);
    radio?.focus();
  }, [picking]);
}

// Asked whether to discard the note, the focus goes to keeping it, so
// that Enter is no slip; keeping it gives the note the focus back.
function useKeepFocus(dialog: RefObject<HTMLDialogElement>, leaving: boolean): void {
  useEffect(() => {
    if (leaving) dialog.current?.querySelector<HTMLButtonElement>(".leaving .keep")?.focus();
  }, [leaving]);
}

// On a phone the dialog takes the part of the screen the on-screen
// keyboard leaves, which 100dvh is not: iOS Safari lays the keyboard over
// the page, and the send button under it could not be scrolled to.
function useSeen(dialog: RefObject<HTMLDialogElement>): void {
  useEffect(() => {
    const d = dialog.current;
    const seen = window.visualViewport;
    if (!d || !seen) return;
    const fit = () => {
      d.style.setProperty("--seen-top", `${seen.offsetTop}px`);
      d.style.setProperty("--seen-height", `${seen.height}px`);
    };
    fit();
    seen.addEventListener("resize", fit);
    seen.addEventListener("scroll", fit);
    return () => {
      seen.removeEventListener("resize", fit);
      seen.removeEventListener("scroll", fit);
    };
  }, []);
}

// The choices not listening are folded while there are some listening.
// Until the fold is opened or closed by hand (`open`), it is open while the
// one picked is among them, as a reply to a worker that has stopped
// listening picks it, and while the filter has text, so that a session
// starting to listen does not fold away a list being narrowed. With nobody
// listening they are the whole list, and are not folded. The one picked
// stays in sight (`held`) when its fold is closed.
function fold(live: Pick[], idle: Pick[], open: boolean | null, filter: string, pick: string): { unfolded: boolean; shown: Pick[]; held: Pick | undefined } {
  const unfolded = live.length === 0 || (open ?? (filter.trim() !== "" || idle.some((p) => p.key === pick)));
  const shown = filtered(idle, filter, pick);
  const held = unfolded ? undefined : idle.find((p) => p.key === pick);
  return { unfolded, shown, held };
}

// What the picker draws from.
interface Picking {
  f: Fields;
  set: <K extends keyof Fields>(key: K) => (value: Fields[K]) => void;
  live: Pick[];
  idle: Pick[];
  unfolded: boolean;
  shown: Pick[];
  held: Pick | undefined;
  filter: string;
  setFilter: (filter: string) => void;
  setOpen: (open: boolean) => void;
  listeners: Listener[] | null;
  now: number;
  picker: RefObject<HTMLFieldSetElement>;
}

// The picker: who it goes to.
function picker(p: Picking) {
  return html`<fieldset class="picker" ref=${p.picker}>
    <legend>to</legend>
    ${p.live.length > 0 && html`<p class="section">listening now</p>`}
    ${p.live.map((c) => html`<${Choice} key=${c.key} p=${c} checked=${p.f.pick === c.key} onPick=${p.set("pick")} now=${p.now} />`)}
    ${p.held && html`<${Choice} key=${p.held.key} p=${p.held} checked=${true} onPick=${p.set("pick")} now=${p.now} />`}
    ${foldButton(p)}
    ${notListening(p)}
    <label class="choice pick"><input type="radio" name="to" value="other" checked=${p.f.pick === "other"} onClick=${() => p.set("pick")("other")} onChange=${() => p.set("pick")("other")} /> <span>another</span></label>
    ${another(p)}
  </fieldset>`;
}

// The line the choices not listening fold behind, which counts them.
function foldButton(p: Picking) {
  return p.live.length > 0 && p.idle.length > 0 && html`<button type="button" class="fold" aria-expanded=${p.unfolded ? "true" : "false"} aria-controls=${p.unfolded ? "not-listening" : undefined} onClick=${() => p.setOpen(!p.unfolded)}>${p.idle.length - (p.held ? 1 : 0)} more, not listening</button>`;
}

// The choices not listening, with their filter, while unfolded.
function notListening(p: Picking) {
  return p.idle.length > 0 && p.unfolded && html`<div id="not-listening">
      ${p.listeners && html`<p class="section">not listening now: it waits until one is</p>`}
      <input type="text" name="filter" class="filter" autocomplete="off" aria-label="filter by name" placeholder="filter by name" value=${p.filter} onInput=${(e: Event) => p.setFilter((e.target as HTMLInputElement).value)} />
      ${p.shown.map((c) => html`<${Choice} key=${c.key} p=${c} checked=${p.f.pick === c.key} onPick=${p.set("pick")} now=${p.now} />`)}
      ${p.shown.length === 0 && html`<p class="context">no worker or label here has “${p.filter.trim()}” in its name</p>`}
    </div>`;
}

// The kind and name of another, while it is picked.
function another(p: Picking) {
  return p.f.pick === "other" && html`<div class="other">
      <select name="kind" value=${p.f.otherKind} onChange=${(e: Event) => p.set("otherKind")((e.target as HTMLSelectElement).value as "worker" | "label")}><option value="worker">worker</option><option value="label">label</option></select>
      <input type="text" name="other" aria-label="its name" value=${p.f.other} onInput=${(e: Event) => p.set("other")((e.target as HTMLInputElement).value)} />
    </div>`;
}

// The buttons under the note: send and close, or, asked whether to discard
// the note, discard and keep writing.
function actions(c: Leaving) {
  return c.leaving
    ? html`<div class="actions leaving" role="alert"><span>discard this note?</span><button type="button" class="primary" onClick=${() => c.dialog.current?.close()}>discard</button><button type="button" class="keep" onClick=${() => {
        c.setLeaving(false);
        c.dialog.current?.querySelector<HTMLTextAreaElement>('textarea[name="body"]')?.focus();
      }}>keep writing</button></div>`
    : html`<div class="actions"><button type="submit" class="primary" aria-keyshortcuts=${SEND_SHORTCUTS}>send <${Hint} keys=${SEND} /></button><button type="button" onClick=${() => leave(c)}>close</button></div>`;
}

// The line that says what case a reply is about, linked to it.
function aboutLine(re: string, about: Case | undefined, onClick: (e: MouseEvent) => void) {
  return re && html`<p class="re">about <a href=${about ? page(about) : pageOf(re)} onClick=${onClick}>${about ? about.title : re}</a></p>`;
}

// The setter of one field of the composer's.
function setter(setF: (update: (was: Fields) => Fields) => void) {
  return <K extends keyof Fields>(key: K) => (value: Fields[K]) => setF((was) => ({ ...was, [key]: value }));
}

// The composer, a dialog over the page: who it goes to, the note, and
// send. There is no title to write, as a note slipped to someone has none:
// the note's first line is the message's title (`title`), and the note is
// its body; and how soon is not asked. `to` and `re` come from the link
// that opened it, as "message this agent" on a case gives them: opened for
// someone, it says who, with a change button that shows the picker, and
// "about" the case; opened for no one, it shows the picker. The note has
// the focus when it opens, and the picker once asked for. Escape, or a
// click beside the dialog, closes it, once the human says so when a note is
// written. A send that is made closes it, and `onSent` says so; one that is
// refused says why, above the send button, where the eye is after pressing
// it, and keeps what was typed. The dialog keeps the focus in it while it is
// open, and gives it back where it was when it closes.
export function Composer({
  to,
  re,
  listeners,
  cases,
  now,
  onSent,
  onClose,
  back = { current: null },
}: {
  to: string;
  re: string;
  listeners: Listener[] | null;
  cases: Case[];
  now: number;
  onSent: (after: Case) => void;
  onClose: () => void;
  // Where the composer puts what back does, which the app calls when back
  // takes its entry off the history (Entry): whether it asks about a note
  // written, rather than closing.
  back?: { current: (() => boolean) | null };
}) {
  // While who listens is not known (null), the choices are one plain list,
  // which says nothing of who listens.
  const offered = picks(listeners || [], cases, now);
  const about = re ? cases.find((c) => c.id === re) : undefined;
  const [f, setF] = useState<Fields>(() => ({ ...opening(to, about, offered), otherKind: "worker", body: "" }));
  // Whether the picker shows: from the start for no one, and for someone
  // once asked for.
  const [picking, setPicking] = useState(!to);
  // Whether the dialog asks before it closes on a note written.
  const [leaving, setLeaving] = useState(false);
  const choices = kept(offered, f.pick);
  const live = choices.filter((p) => p.live);
  const idle = choices.filter((p) => !p.live);
  // Whether the fold of the choices not listening was opened or closed by
  // hand, and the filter's text (fold).
  const [open, setOpen] = useState<boolean | null>(null);
  const [filter, setFilter] = useState("");
  const { unfolded, shown, held } = fold(live, idle, open, filter, f.pick);
  const [refusal, setRefusal] = useState("");
  const sending = useRef<Guard | null>(null);
  const dialog = useRef<HTMLDialogElement>(null);
  const pickerRef = useRef<HTMLFieldSetElement>(null);
  const set = setter(setF);
  useShown(dialog);
  usePickerFocus(pickerRef, picking, to);
  useKeepFocus(dialog, leaving);
  useSeen(dialog);
  // Whether a press on the dialog started on its backdrop (dialogEvents).
  const down = useRef(false);
  const c: Leaving = { f, leaving, setLeaving, dialog };
  back.current = () => backed(c);
  const send: Sending = { f, choices, re, sending, dialog, setRefusal, onSent };

  return html`<dialog class="options compose-dialog" id="compose-dialog" aria-labelledby="compose-title" ref=${dialog}
    ...${dialogEvents(c, down)}
    onClose=${onClose}><div id="compose">
<header class="case-head">
  <h1 id="compose-title">message an agent</h1>
</header>
<form class="respond compose" data-write="message" onSubmit=${(e: Event) => submit(send, e)} onKeyDown=${onKey}>
  ${!picking && html`<p class="to">to <strong>${named(f.pick, f)}</strong> <button type="button" class="change" onClick=${() => setPicking(true)}>change</button></p>`}
  ${picking && picker({ f, set, live, idle, unfolded, shown, held, filter, setFilter, setOpen, listeners, now, picker: pickerRef })}
  ${aboutLine(re, about, (e) => toCase(c, e))}
  <label>note <${Text} name="body" rows="5" value=${f.body} onInput=${set("body")} autofocus /></label>
  <p class="error" role="alert" hidden=${!refusal}>${refusal}</p>
  ${actions(c)}
</form>
</div></dialog>`;
}
