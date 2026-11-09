// A case (IN-02), its form (IN-03), and what a send does with a refusal
// (IN-04, IN-05) or a change that lands while the form is open (IN-06). A
// case an agent has takes the human's withdraw (ST-14). A message the human
// sent (IN-13) takes an edit and a withdraw until an agent picks it up, and
// then says who has it; any case says what it is about, and the cases about
// it, and one with a worker offers to message that agent.
import { useEffect, useLayoutEffect, useRef, useState } from "preact/hooks";
import * as api from "./api.js";
import { html } from "./html.js";
import { codeBlock, isDiff, wrapped } from "./code.js";
import { link } from "./links.js";
import { inlineMarkdown, markdown } from "./markdown.js";
import { draw } from "./diagram.js";
import { followUp } from "./followup.js";
import { since } from "./fresh.js";
import { Hint, SEND, SEND_SHORTCUTS } from "./keys.js";
import { Properties } from "./props.js";
import { Doing, addressee, listenerOf } from "./lists.js";
import { CASE_REF, page, pageOf } from "./ids.js";
import { compose } from "./routes.js";
import { answered, thread, who } from "./thread.js";
import { age, stamp } from "./time.js";
import type { ComponentChildren } from "preact";
import type { What } from "./api.js";
import type { FollowUp } from "./followup.js";
import type { Answer, ApprovalRow, Case, EventData, Listener, RowVerdict, State, Time, Write } from "./types.js";

// What a form was drawn from: see drawnFrom.
interface Drawn {
  revision: number;
  amendSeq: number;
  state: State;
  seq: number;
}

// A write's fields, less the revision the case view adds.
type Body = Omit<Write, "revision">;

// What the case view writes: every write but a message, which is not a
// write to a case.
type Sends = Exclude<What, "message">;

// What the answer form holds: the choices made and the text typed. `file`
// is a briefing's left-out items ticked to file, by number from 1.
export interface Fields {
  choice: string;
  verdicts: Record<string, string>;
  rowNotes: Record<string, string>;
  signoff: string;
  briefing: string;
  file: number[];
  stuck: string;
  text: string;
  note: string;
}

// What an age says in the case and its thread: "now", or "5m ago".
function old(at: Time | undefined, now: number): string {
  const a = age(at, now);
  return a === "now" ? "now" : `${a} ago`;
}

// `parse`, remembered by what it is given: the case view is drawn again on
// every keystroke in its form and every event of the store, and its texts
// change only when the case does. The arguments are the key, so they must
// hold all the HTML depends on that can change while the page is open: the
// text, and the wrap option, which every code block, a script's or one in
// markdown, draws on its button (code.js). The page's origin, for links,
// does not change. The last `size` held are kept.
function remembered<A extends unknown[]>(parse: (...args: A) => string, size = 128): (...args: A) => string {
  const held = new Map<string, string>();
  return (...args) => {
    const key = JSON.stringify(args);
    let html = held.get(key);
    if (html === undefined) html = parse(...args);
    else held.delete(key);
    held.set(key, html);
    if (held.size > size) held.delete(held.keys().next().value!);
    return html;
  };
}
const markdownOf = remembered((text: string | undefined, _wrap: boolean) => markdown(text));
// Inline markdown has no code blocks, so no wrap option.
const inlineMarkdownOf = remembered(inlineMarkdown);
const scriptOf = remembered((text: string, wrap: boolean) => codeBlock(text, isDiff(text), wrap));

// Markdown, with any diagram in it drawn once it is in the page
// (diagram.js).
function Markdown({ text, class: classes = "body" }: { text: string | undefined; class?: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const body = markdownOf(text, wrapped());
  useEffect(() => {
    if (ref.current) draw(ref.current);
  }, [body]);
  return html`<div class=${classes} ref=${ref} dangerouslySetInnerHTML=${{ __html: body }} />`;
}

// A row's script as a code block (code.js), drawn as a diff when it is one.
function Script({ text }: { text: string }) {
  return html`<div class="code" dangerouslySetInnerHTML=${{ __html: scriptOf(text, wrapped()) }} />`;
}

// A link of the case's or a row's: an address the page may link to
// (links.js) is a link to a new tab, anything else, such as a file's path,
// is text (IN-02).
function Link({ href }: { href: string }) {
  const url = link(href);
  return url ? html`<a href=${url.href} target="_blank" rel="noopener noreferrer">${href}</a>` : href;
}

// Plain text with each `case:ID` in it a link to that case's page, as
// markdown's are (markdown.js): a thread line says what an event said as
// text.
export function withCaseLinks(text: string) {
  const parts = [];
  let at = 0;
  for (const m of text.matchAll(CASE_REF)) {
    parts.push(text.slice(at, m.index), html`<a class="case-ref" href=${pageOf(m[1]!)}>${m[0]}</a>`);
    at = m.index + m[0].length;
  }
  parts.push(text.slice(at));
  return parts;
}

// The boxes held in view while the case scrolls (style.css): the properties
// column, and the answer form beside the case.
const HELD = "#case > .props, .case-main > .answer";

// How far `box`, held in view by position: sticky, sits below where it is
// held, its --lift: until its pane has scrolled far enough to hold it, it
// sits lower, under the case's head, and style.css caps it that much
// shorter, so its foot, with the form's send button, is in view wherever the
// case is scrolled. A box not held has none (null). It only reads the page,
// so that every box is measured before any is changed.
function lift(box: HTMLElement): number | null {
  const style = getComputedStyle(box);
  if (style.position !== "sticky") return null;
  // Held against the case's pane where it scrolls on its own, else the window.
  const work = box.closest<HTMLElement>(".split > .work");
  const port = work && getComputedStyle(work).overflowY === "auto" ? work.getBoundingClientRect().top + work.clientTop : 0;
  const below = box.getBoundingClientRect().top - (port + (parseFloat(style.top) || 0));
  return Math.max(0, Math.round(below));
}

// Keeps each held box's --lift (lift) as the case's pane or the window
// scrolls, the window changes size, the case's head changes height (a font
// that loads late, the text size, an amended title), and the case is drawn
// again.
function useLift(): void {
  const soon = useRef<() => void>(() => {});
  // Before the first paint, so a box is never drawn at its full cap first.
  useLayoutEffect(() => {
    let frame = 0;
    const all = () => {
      cancelAnimationFrame(frame);
      frame = 0;
      const boxes = [...document.querySelectorAll<HTMLElement>(HELD)];
      const lifts = boxes.map(lift);
      boxes.forEach((box, i) => {
        const by = lifts[i];
        if (by === null) box.style.removeProperty("--lift");
        else box.style.setProperty("--lift", `${by}px`);
      });
    };
    // Once a frame at most.
    soon.current = () => {
      if (!frame) frame = requestAnimationFrame(all);
    };
    all();
    // A scroll does not bubble: the capture sees the pane's and the window's,
    // and the others' (the list's, a box's own), which move no held box.
    const scrolled = (e: Event) => {
      if (e.target === document || (e.target instanceof Element && e.target.matches(".split > .work"))) soon.current();
    };
    document.addEventListener("scroll", scrolled, { capture: true, passive: true });
    addEventListener("resize", soon.current);
    // The head sets where the held boxes start; a change to its height is
    // seen after layout and before paint, so no box is drawn at its old lift.
    const head = document.querySelector("#case > .case-head");
    const grown = new ResizeObserver(all);
    if (head) grown.observe(head);
    const was = soon.current;
    return () => {
      cancelAnimationFrame(frame);
      grown.disconnect();
      document.removeEventListener("scroll", scrolled, { capture: true });
      removeEventListener("resize", was);
    };
  }, []);
  useEffect(() => soon.current());
}

// What a form drawn from case `c` carries, its revision and amend seq
// (IN-05, ST-04), with the state it was drawn in and the highest seq among
// the case's rows, skipped ones included: a new row is numbered above every
// row the case has, so the events after it are the ones since.
export function drawnFrom(c: Case): Drawn {
  return { revision: c.revision, amendSeq: c.amend_seq || 0, state: c.state, seq: c.last_seq };
}

// What a form drawn as `was` carries once the case is `c`.
//
// A case in another state is shown in that state, with the form it takes,
// at its revision now, whatever `since` (fresh.ts) says. That is safe only because no
// form keeps what was typed across a change of state: the answer form is
// drawn only while the case is open, the withdraw is keyed by the state and
// the edit of a message by its amend seq, so each is drawn afresh, empty,
// and the human sees the new state before sending (IN-06). A form that kept
// its draft across states would send it against a case the human has not
// seen, such as one answered elsewhere, reopened and answered again between
// two reads; such a form must follow `since` instead, and take the new
// revision only when it says "moved" and the form fits the new state.
//
// A case in the state it was drawn in keeps its form as it was left; the
// form takes the new revision only when `since` says "unchanged", such as
// after a note or the human's own edit of a message. After anything else,
// such as an amend that changed the question, or a park and a resume
// between two reads, it keeps its revision, so that its send is refused as
// stale (IN-05).
export function redrawn(was: Drawn, c: Case): Drawn {
  const now = drawnFrom(c);
  if (c.state !== was.state) return now;
  return since(c, was) === "unchanged" ? now : was;
}

// The case `c`, with the form its state takes. `onSent` is told of a write
// that was made, with the case after it; `onCase` of the case as it is now,
// when a send was refused. `cases` are the others the page read, to name
// what `c` is about and list the cases about it, and `listeners` the workers
// listening now, to say what the one that has a message is doing.
export function CaseView({
  c,
  now,
  onSent,
  onCase,
  cases = [],
  listeners = [],
}: {
  c: Case;
  now: number;
  onSent: (what: Sends, after: Case) => void;
  onCase: (current: Case) => void;
  cases?: Case[];
  listeners?: Listener[];
}) {
  const [refusal, setRefusal] = useState("");
  const [drawn, setDrawn] = useState(() => drawnFrom(c));
  const sending = useRef<object | null>(null);
  useLift();

  // The form follows the case as `redrawn` says.
  useEffect(() => {
    setDrawn((was) => redrawn(was, c));
  }, [c.revision, c.state, c.amend_seq]);

  const send = (what: Sends, body: Body) => sendOnce({ id: c.id, what, body, drawn, setDrawn, setRefusal, sending, onSent, onCase });

  const fu = followUp(c);
  // What comes before the answer box and what comes after it are one block
  // each: where the form is beside the case, it spans the two, and the
  // second grows to the form's height (style.css), so nothing that either
  // holds is pushed below the form, whatever the case shows.
  return html`<div id="case">
<header class="case-head">
  <h1>${c.title}</h1>
  ${head(c, cases)}
</header>
<${Properties} c=${c} now=${now} />
<div class="case-main">
<div class="before-answer">
<article class="case">
  ${article(c, fu, now)}
</article>
${messageAfter(c, listeners, now)}
</div>
<div class="answer">
<p class="error" role="alert" id="refusal" hidden=${!refusal}>${refusal}</p>
${answerBox(c, fu, send, setRefusal)}
</div>
<div class="after-answer">
${(c.state === "answered" || c.state === "pickedup" || c.state === "sent") && html`<${Withdraw} key=${`${c.id} ${c.state}`} send=${send} sent=${c.state === "sent"} message=${c.kind === "message"} />`}
<${About} id=${c.id} cases=${cases} of=${c} />
${c.kind !== "message" && html`<${Thread} c=${c} now=${now} />`}
</div>
</div>
</div>`;
}

// What a send of case `id` needs from the view that sends it: the form as
// drawn, and how to say what came of it.
export interface Sending {
  id: string;
  what: Sends;
  body: Body;
  drawn: Drawn;
  setDrawn: (next: Drawn | ((was: Drawn) => Drawn)) => void;
  setRefusal: (refusal: string) => void;
  sending: { current: object | null };
  onSent: (what: Sends, after: Case) => void;
  onCase: (current: Case) => void;
}

// Sends `what` of `body`, once: a send while one is out does nothing,
// until it is answered or 10 s have passed (IN-09). `sending` holds the
// send that is out, and only that send lets it go: one slower than 10 s
// does not, when it comes back, let go of the send that went after it.
export async function sendOnce({ id, what, body, drawn, setDrawn, setRefusal, sending, onSent, onCase }: Sending): Promise<void> {
  if (sending.current) return;
  const self = {};
  sending.current = self;
  const done = () => {
    if (sending.current === self) sending.current = null;
  };
  const release = setTimeout(done, 10000);
  const sent: Write = { revision: drawn.revision, ...body };
  if (what === "answer") sent.answer = { ...body.answer, amend_seq: drawn.amendSeq };
  try {
    const { after, refusal, current } = await api.send(id, what, sent);
    if (after) {
      setRefusal("");
      // The forms go on from the case as this write left it: after the
      // edit of a message the page stays on it, and a withdraw or another
      // edit may be sent before the stream redraws it (IN-13). A redraw
      // that came first, from a later event, is kept.
      setDrawn((was) => (after.revision > was.revision ? drawnFrom(after) : was));
      onSent(what, after);
      return;
    }
    setRefusal(refusal);
    // The case as it is now: the form, which keeps what was typed, takes
    // its revision, as it has now been shown (IN-05), unless a later write
    // or redraw has already moved it past that case.
    if (current) {
      setDrawn((was) => (current.revision >= was.revision ? drawnFrom(current) : was));
      onCase(current);
    }
  } finally {
    clearTimeout(release);
    done();
  }
}

// The head of case `c` under its title: a briefing's landing, a message's addressee, what
// the case is about, among `cases`, its context and brief, and a link to
// message the agent that asked it.
function head(c: Case, cases: Case[]) {
  return html`${c.kind === "briefing" && html`<p class=${c.landed ? "landed-mark landed" : "landed-mark gated"}>${c.landed ? "landed" : "waiting to land"}</p>`}
  ${c.kind === "message" && html`<p class="to">message to <strong>${addressee(c)}</strong></p>`}
  ${c.re && html`<${About} id=${c.re} cases=${cases} />`}
  ${c.context && html`<p class="context">${c.context}</p>`}
  ${c.brief && html`<p class="brief"><span class="label">brief</span> ${c.brief}</p>`}
  ${c.kind !== "message" && c.worker && html`<p class="message-agent"><a href=${compose(c.worker, c.id)}>message ${c.worker}</a></p>`}`;
}

// The article of case `c`: its body, or follow-up `fu` when it is one, then
// its left-out items and its links.
function article(c: Case, fu: FollowUp | null, now: number) {
  const links = c.links || [];
  return html`${fu ? html`<${FollowUpFirst} c=${c} fu=${fu} now=${now} />` : html`<${Markdown} text=${c.body} />`}
  ${(c.left_out || []).length > 0 && html`<section class="left-out" aria-labelledby="left-out-head"><h2 class="section" id="left-out-head">left out</h2><ol>${c.left_out!.map((item) => html`<li>${item}</li>`)}</ol></section>`}
  ${links.length > 0 && html`<section class="case-links" aria-labelledby="links-head"><h2 class="section" id="links-head">links</h2><ul class="links">${links.map((href) => html`<li><${Link} href=${href} /></li>`)}</ul></section>`}`;
}

// What a message says after its article: its outcome once closed, who
// picked it up, and who withdrew it.
function messageAfter(c: Case, listeners: Listener[], now: number) {
  if (c.kind !== "message") return null;
  return html`${c.state === "closed" && c.close && html`<section class="outcome" aria-labelledby="outcome-head"><h2 class="section" id="outcome-head">outcome</h2><${Markdown} text=${c.close.outcome} />${(c.close.links || []).length > 0 && html`<ul class="links">${c.close.links!.map((href) => html`<li><${Link} href=${href} /></li>`)}</ul>`}</section>`}
${c.pickup && html`<${Taken} c=${c} listeners=${listeners} now=${now} />`}
${c.state === "withdrawn" && html`<${Withdrawn} c=${c} now=${now} />`}`;
}

// The form case `c` takes in its state: the answer, with follow-up `fu`'s
// answer as the last, the resume of a parked case, or the edit of a sent
// message.
function answerBox(c: Case, fu: FollowUp | null, send: (what: Sends, body: Body) => Promise<void>, refuse: (refusal: string) => void) {
  return html`${c.state === "open" && html`<${Respond} key=${c.id} c=${c} last=${fu ? fu.answer.data : undefined} send=${send} refuse=${refuse} />`}
${c.state === "parked" && html`<form class="respond" data-write="resume" onSubmit=${(e: Event) => {
    e.preventDefault();
    send("resume", {});
  }}><div class="actions"><button type="submit">resume</button></div></form>`}
${c.state === "sent" && html`<${Edit} key=${`${c.id} ${c.amend_seq || 0}`} c=${c} send=${send} refuse=${refuse} />`}`;
}

// What case `about` is about, `id`, by its title when the page read it and
// by its id when not, such as a case pruned since; or, given `of`, the cases
// the page read that are about `of`, with their state, or nothing when none
// are.
function About({ id, cases, of }: { id: string; cases: Case[]; of?: Case }) {
  if (!of) {
    const found = cases.find((c) => c.id === id);
    return html`<p class="re">about <a href=${found ? page(found) : pageOf(id)}>${found ? found.title : id}</a></p>`;
  }
  const about = cases.filter((c) => c.re === id);
  if (!about.length) return null;
  return html`<section class="follow-ups" aria-labelledby="follow-ups-head"><h2 class="section" id="follow-ups-head">about this ${of.kind === "message" ? "message" : "case"}</h2><ul class="flights">${about.map(
    (c) => html`<li key=${c.id}><a href=${page(c)}>${c.title}</a><span class="flight">${c.kind} · ${c.state}</span></li>`,
  )}</ul></section>`;
}

// Who took the message `c` back, when, and why, if they said: a message's
// page has no thread (a message's is only its send and what the agent did,
// which the page says), and this is where the thread said it.
function Withdrawn({ c, now }: { c: Case; now: number }) {
  const withdraw = (c.events || []).filter((e) => e.event === "withdraw").pop();
  const by = withdraw && who(withdraw.actor);
  const reason = withdraw && (withdraw.data || {}).reason;
  return html`<p class="taken withdrawn">withdrawn${by ? html` by <strong>${by}</strong>` : ""}${withdraw && withdraw.at ? `, ${old(withdraw.at, now)}` : ""}${reason ? html`: <span class="reason">${reason}</span>` : ""}</p>`;
}

// Who picked up the message `c`, and when, with what that worker says it is
// doing while it listens and still has the message: once the message is
// closed or withdrawn, what the worker does is other work.
function Taken({ c, listeners, now }: { c: Case; listeners: Listener[]; now: number }) {
  const pickup = (c.events || []).filter((e) => e.event === "pickup").pop();
  const by = (c.pickup && c.pickup.by) || (pickup && who(pickup.actor)) || "an agent";
  const at = c.pickup && c.pickup.picked_up_at;
  const l = c.state === "pickedup" ? listenerOf(listeners, by, c.worker) : undefined;
  return html`<p class="taken">picked up by <strong>${by}</strong>${at ? `, ${old(at, now)}` : ""}<${Doing} l=${l} now=${now} /></p>`;
}

// The human's edit of a message no agent has picked up: its body, as the
// human's amend, which a picked-up message no longer takes. The title stays:
// the amend has none. Folded away, as it is not the page's main work. The
// case view keys it by the message's amend seq, so it is drawn afresh with
// the body when an edit made elsewhere replaces the body, and not for other
// changes, such as a label added, which keep what was typed (IN-06). The
// form says when its body differs from the message's, as the page's check
// for typed text reads it.
function Edit({
  c,
  send,
  refuse,
}: {
  c: Case;
  send: (what: Sends, body: Body) => Promise<void>;
  refuse: (problem: string) => void;
}) {
  const [body, setBody] = useState(c.body || "");
  function submit(e: Event): void {
    e.preventDefault();
    const text = body.trim();
    if (!text) {
      refuse("write the message, or withdraw it");
      return;
    }
    if (text === (c.body || "").trim()) {
      refuse("the message is as it was, so there is nothing to save");
      return;
    }
    send("amend", { body: text });
  }
  const edited = body.trim() !== (c.body || "").trim();
  return html`<form class="respond" data-write="amend" data-edited=${edited ? "" : undefined} onSubmit=${submit}><details class="edit">
  <summary>edit this message</summary>
  <p class="context">until an agent picks it up. the title stays: to change it, withdraw the message and send another.</p>
  <label>message <${Text} name="body" rows="4" value=${body} onInput=${setBody} /></label>
  <div class="actions"><button type="submit" id="respond-edit">save</button></div>
</details></form>`;
}

// A follow-up (followup.js), drawn follow-up first, so that it is not read as
// the question asked again: the agent's notes since the answer, newest first,
// then that answer and its pickup, then the case as it was first asked,
// folded.
function FollowUpFirst({ c, fu, now }: { c: Case; fu: FollowUp; now: number }) {
  const pickup = (c.events || []).find((e) => e.seq > fu.answer.seq && e.event === "pickup");
  const by = pickup && ((pickup.data || {}).by || who(pickup.actor));
  return html`<section class="follow-up" aria-label="follow-up">
  ${[...fu.notes].reverse().map((note) => html`<div key=${note.seq} class="follow-up-note">
    <div class="meta"><span class="badge">new since your answer</span><span>${who(note.actor) || note.author} followed up</span>${note.at && html`<span class="age">${stamp(note.at)}, ${old(note.at, now)}</span>`}</div>
    <${Markdown} text=${(note.data || {}).body} />
  </div>`)}
  <div class="previous-answer">
    <div class="meta"><span>your previous answer</span>${fu.answer.at && html`<span class="age">${stamp(fu.answer.at)}, ${old(fu.answer.at, now)}</span>`}</div>
    ${answered(c, fu.answer.data).map((line) => html`<p>${line}</p>`)}
    ${pickup && html`<p class="picked-up">picked up${by ? ` by ${by}` : ""}${pickup.at ? `, ${old(pickup.at, now)}` : ""}</p>`}
  </div>
  <details class="original"><summary>the question as first asked${c.opened_at ? `, ${old(c.opened_at, now)}` : ""}</summary><${Markdown} text=${c.body} /></details>
</section>`;
}

// Whether the browser sizes a textarea to its text itself.
const sized = typeof CSS !== "undefined" && CSS.supports && CSS.supports("field-sizing", "content");

// A textarea that grows with what is typed in it (IN-03): the stylesheet's
// field-sizing where the browser has it, and its scroll height elsewhere.
export function Text({
  name,
  rows,
  value,
  onInput,
  required = false,
  shortcut,
  autofocus = false,
}: {
  name: string;
  rows: string;
  value: string;
  onInput: (value: string) => void;
  required?: boolean;
  shortcut?: string;
  // Whether the field takes the focus when its dialog opens.
  autofocus?: boolean;
}) {
  const ref = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (sized || !el) return;
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight + el.offsetHeight - el.clientHeight}px`;
  }, [value]);
  return html`<textarea ref=${ref} name=${name} rows=${rows} required=${required} autofocus=${autofocus} aria-keyshortcuts=${shortcut} value=${value} onInput=${(e: Event) => onInput((e.target as HTMLTextAreaElement).value)} />`;
}

// What `fields`, a form's verdicts or row notes, holds for the row `id`:
// its own properties only, as a row id can be "constructor" or "__proto__".
function of(fields: Record<string, string>, id: string): string | undefined {
  return Object.hasOwn(fields, id) ? fields[id] : undefined;
}

// Whether the options pre-select approve for an approval's rows (IN-10).
function preselected(): boolean {
  return typeof window !== "undefined" && !!window.casesPrefs && window.casesPrefs.read("approve") === "on";
}

// The verdict row `id` has in a form: the one chosen for it, else approve
// where the options pre-select it, else none. So a row an amend adds starts
// pre-selected too, and a row already chosen keeps its choice.
export function verdictOf(verdicts: Record<string, string>, id: string, preselect: boolean): string | undefined {
  return of(verdicts, id) || (preselect ? "approve" : undefined);
}

// Whether the human carries out `row`, so its approve says the step is done
// (case i2lDf). Done is never pre-selected: the human says it.
function byHuman(row: ApprovalRow): boolean {
  return row.by === "human";
}

// The words of a row's three verdicts: what approve, hold and reject say,
// with a hint each. On the human's row, approve is done.
function verdictWords(row: ApprovalRow): [string, string][] {
  return byHuman(row)
    ? [["done", "you did it"], ["hold", "not yet; ask again later"], ["won't do", "you won't do this step"]]
    : [["approve", "run it"], ["hold", "not now; ask again later"], ["don't run", "never run this row"]];
}

// One choice of a set of radios, with the number key that chooses it
// (keys.js), if it has one. A click picks it even when it is already
// checked, which makes a pre-selected verdict a chosen one.
function Choice({
  name,
  value,
  checked,
  onPick,
  required = false,
  n,
  children,
}: {
  name: string;
  value: string;
  checked: boolean;
  onPick: (value: string) => void;
  required?: boolean;
  n?: number;
  children?: ComponentChildren;
}) {
  const key = n && n <= 9 ? String(n) : undefined;
  return html`<label class="choice"><input type="radio" name=${name} value=${value} required=${required} checked=${checked} aria-keyshortcuts=${key} onClick=${() => onPick(value)} onChange=${() => onPick(value)} /> ${children}${key && html`<${Hint} keys=${[key]} />`}</label>`;
}

// The answer to case `c` that the form's fields `f` give, or the problem
// that keeps it from being sent: a form missing its response is never sent
// (IN-03). Notes and text are trimmed: the store keeps text as it is sent.
// With `preselect`, a row with no verdict chosen is sent as approve. A
// decision's other and a signoff's request for changes need the note, as a
// briefing sent back does (IN-14): the store refuses them without one.
export function response(c: Case, f: Fields, preselect: boolean): { answer: Answer; problem?: never } | { answer?: never; problem: string } {
  const note = f.note.trim();
  const given = byKind(c, f, note, preselect);
  if (typeof given === "string") return { problem: given };
  return { answer: note ? { note, ...given } : given };
}

// What a kind's answer holds besides the note, or the problem that keeps it
// from being sent.
type Given = Answer | string;

// The response the fields give to case `c`, by its kind; `note` is the
// note, trimmed. A kind with no response of its own, a message, which is
// never open, gives none.
function byKind(c: Case, f: Fields, note: string, preselect: boolean): Given {
  switch (c.kind) {
    case "decision":
      return decided(f, note);
    case "approval":
      return judged(c, f, preselect);
    case "signoff":
      return signedOff(f, note);
    case "stuck":
      return unstuck(f);
    case "question":
      return replied(f);
    case "fyi":
      return { ack: true };
    case "briefing":
      return briefed(c, f, note);
  }
  return {};
}

// A decision's option, or other with the note.
function decided(f: Fields, note: string): Given {
  if (!f.choice) return "choose an option";
  if (f.choice === "other" && !note) return "other needs a note: say what you want instead";
  return f.choice === "other" ? { other: true } : { choice: Number(f.choice) };
}

// An approval's verdict on each of its rows, with the row's note.
function judged(c: Case, f: Fields, preselect: boolean): Given {
  const rows: RowVerdict[] = [];
  for (const row of c.rows || []) {
    const verdict = verdictOf(f.verdicts, row.id, preselect && !byHuman(row));
    if (!verdict) {
      const [approve, hold, reject] = verdictWords(row).map(([word]) => word);
      return `choose ${approve}, ${hold} or ${reject} for "${row.label}"`;
    }
    const rowNote = (of(f.rowNotes, row.id) || "").trim();
    rows.push((rowNote ? { id: row.id, verdict, note: rowNote } : { id: row.id, verdict }) as RowVerdict);
  }
  return { rows };
}

// A signoff's accept, or its request for changes with the note.
function signedOff(f: Fields, note: string): Given {
  if (!f.signoff) return "choose accept or request changes";
  if (f.signoff === "changes" && !note) return "requesting changes needs a comment: say what to change";
  return { signoff: f.signoff as Answer["signoff"] & string };
}

// A stuck case's guidance, or its drop.
function unstuck(f: Fields): Given {
  const text = f.text.trim();
  // Guidance typed without its button chosen still means guidance.
  const choice = f.stuck || (text ? "text" : "");
  if (choice === "drop") return { drop: true };
  if (choice === "text" && text) return { text };
  if (choice === "text") return "write the guidance, or choose drop";
  return "choose guidance or drop, or park it";
}

// A question's reply.
function replied(f: Fields): Given {
  const text = f.text.trim();
  if (!text) return "write the reply";
  return { text };
}

// A briefing's accept, or its send back with the note, and the left-out
// items ticked to file, in order.
function briefed(c: Case, f: Fields, note: string): Given {
  if (!f.briefing) return "choose accept or send back";
  if (f.briefing === "send_back" && !note) return "sending back needs a reason: say what to fix";
  const answer: Answer = { briefing: f.briefing as Answer["briefing"] & string };
  // A tick past the list, from before an amend shortened it, files nothing.
  const file = f.file.filter((n) => n <= (c.left_out || []).length).sort((a, b) => a - b);
  if (file.length) answer.file = file;
  return answer;
}

// A choice's text with its marks: "recommended" on the option the agent
// recommends, which any choice may go against, and "your last pick" on the
// one a follow-up's previous answer chose, so that it is not chosen again by
// habit. The marks flow with the text.
function marked(text: string, recommended: boolean, picked: boolean): ComponentChildren {
  if (!recommended && !picked) return text;
  return html`<span>${text}${recommended && html` <span class="badge recommended">recommended</span>`}${picked && html` <span class="badge last-pick">your last pick</span>`}</span>`;
}

// What the fields of the answer form draw from: the case `c`, the form's
// fields `f`, the answer a follow-up reopened, whether approve is
// pre-selected, and the setters Respond keeps its state with.
interface Form {
  c: Case;
  f: Fields;
  last?: EventData | undefined;
  preselect: boolean;
  set: (key: Exclude<keyof Fields, "verdicts" | "rowNotes" | "file">) => (value: string) => void;
  setIn: (key: "verdicts" | "rowNotes", id: string) => (value: string) => void;
  tick: (n: number) => (e: Event) => void;
}

// The fields of the answer form for the case's kind, above its note: none
// for an fyi, which is acknowledged, or a kind the form has no fields for.
function fieldsOf(form: Form): ComponentChildren {
  switch (form.c.kind) {
    case "decision":
      return decisionFields(form);
    case "approval":
      return (form.c.rows || []).map((row) => approvalRow(form, row));
    case "signoff":
      return signoffFields(form);
    case "briefing":
      return briefingFields(form);
    case "stuck":
      return stuckFields(form);
    case "question":
      return questionFields(form);
  }
  return null;
}

// A decision's options, numbered, then other.
function decisionFields({ c, f, last, set }: Form): ComponentChildren {
  return html`<fieldset>
  <legend>choose one</legend>
  ${(c.options || []).map((option, i) => html`<${Choice} name="choice" value=${String(i + 1)} n=${i + 1} required checked=${f.choice === String(i + 1)} onPick=${set("choice")}>${marked(option, c.recommend === i + 1, !!last && last.choice === i + 1)}<//>`)}
  <${Choice} name="choice" value="other" n=${(c.options || []).length + 1} required checked=${f.choice === "other"} onPick=${set("choice")}>${marked("other, see note", false, !!last && !!last.other)}<//>
</fieldset>`;
}

// An approval's row: what it is, its three verdicts, and its own note.
function approvalRow({ f, preselect, setIn }: Form, row: ApprovalRow): ComponentChildren {
  const verdict = `verdict.${row.id}`;
  const pick = setIn("verdicts", row.id);
  const human = byHuman(row);
  const chosen = verdictOf(f.verdicts, row.id, preselect && !human);
  // A row whose verdict is only pre-selected, which the number keys
  // treat as having none (keys.js).
  const preset = !of(f.verdicts, row.id) && !!chosen;
  const words = verdictWords(row);
  return html`<fieldset class="row" key=${row.id} data-preset=${preset || undefined} data-by=${row.by || undefined}>
  <legend class="row-title">${row.label}${human && html` <span class="badge by-human" title="a step you carry out">you</span>`}</legend>
  ${row.note && html`<p class="context row-note" dangerouslySetInnerHTML=${{ __html: inlineMarkdownOf(row.note) }} />`}
  <p><${Link} href=${row.link || ""} /></p>
  <${Script} text=${row.script || ""} />
  <div class="verdicts">
    ${(["approve", "hold", "reject"] as const).map(
      (value, i) =>
        html`<${Choice} name=${verdict} value=${value} n=${i + 1} required checked=${chosen === value} onPick=${pick}>${words[i]![0]} <span class="hint">${words[i]![1]}</span><//>`,
    )}
  </div>
  <label>note on this row <${Text} name=${`note.${row.id}`} rows="1" value=${of(f.rowNotes, row.id) || ""} onInput=${setIn("rowNotes", row.id)} /></label>
</fieldset>`;
}

// A signoff's accept and request for changes.
function signoffFields({ f, set }: Form): ComponentChildren {
  return html`<fieldset>
  <legend>sign off</legend>
  <${Choice} name="signoff" value="accept" n=${1} required checked=${f.signoff === "accept"} onPick=${set("signoff")}>accept<//>
  <${Choice} name="signoff" value="changes" n=${2} required checked=${f.signoff === "changes"} onPick=${set("signoff")}>request changes (comment required)<//>
</fieldset>`;
}

// A briefing's accept and send back, then a tick box per left-out item.
function briefingFields({ c, f, set, tick }: Form): ComponentChildren {
  return html`<fieldset>
  <legend>judge the work</legend>
  <${Choice} name="briefing" value="accept" n=${1} required checked=${f.briefing === "accept"} onPick=${set("briefing")}>accept<//>
  <${Choice} name="briefing" value="send_back" n=${2} required checked=${f.briefing === "send_back"} onPick=${set("briefing")}>send back (reason required)<//>
</fieldset>
${(c.left_out || []).length > 0 && html`<fieldset class="file">
  <legend>file what was left out</legend>
  ${c.left_out!.map((item, i) => html`<label class="choice"><input type="checkbox" name="file" value=${String(i + 1)} checked=${f.file.includes(i + 1)} onChange=${tick(i + 1)} /> ${item}</label>`)}
</fieldset>`}`;
}

// A stuck case's guidance, with its text, and its drop.
function stuckFields({ f, set }: Form): ComponentChildren {
  return html`<fieldset>
  <legend>unblock</legend>
  <${Choice} name="stuck" value="text" n=${1} checked=${f.stuck === "text"} onPick=${set("stuck")}>give guidance<//>
  <label>guidance <${Text} name="text" rows="4" value=${f.text} onInput=${set("text")} /></label>
  <${Choice} name="stuck" value="drop" n=${2} checked=${f.stuck === "drop"} onPick=${set("stuck")}>drop it<//>
</fieldset>`;
}

// A question's reply.
function questionFields({ f, set }: Form): ComponentChildren {
  return html`<label>reply <${Text} name="text" rows="4" required value=${f.text} onInput=${set("text")} /></label>`;
}

// The form that answers an open case, shaped by its kind. Its fields keep
// what was typed and chosen while the case is redrawn. On a follow-up,
// `last` is the answer it reopened.
function Respond({
  c,
  last,
  send,
  refuse,
}: {
  c: Case;
  last?: EventData | undefined;
  send: (what: Sends, body: Body) => Promise<void>;
  refuse: (problem: string) => void;
}) {
  const [f, setF] = useState<Fields>({ choice: "", verdicts: {}, rowNotes: {}, signoff: "", briefing: "", file: [], stuck: "", text: "", note: "" });
  // Read once, so an option changed while the form is open leaves it be.
  const [preselect] = useState(preselected);
  const { set, setIn, tick } = setters(setF);
  // An amend that replaces a briefing's left-out items numbers them anew:
  // the ticks made against the old list go, rather than file other items.
  const items = JSON.stringify(c.left_out || []);
  const ticked = useRef(items);
  useEffect(() => {
    if (ticked.current === items) return;
    ticked.current = items;
    setF((was) => ({ ...was, file: [] }));
  }, [items]);

  const { submit, drop, park } = sends(c, f, preselect, send, refuse);
  const fields = fieldsOf({ c, f, last, preselect, set, setIn, tick });
  return html`<form class="respond" data-write="answer" onSubmit=${submit}>
  ${fields}
  ${noteField(c, f, set)}
  ${actions(c, drop, park)}
</form>`;
}

// The form's setters of its fields, over Respond's `setF`.
function setters(setF: (next: (was: Fields) => Fields) => void): Pick<Form, "set" | "setIn" | "tick"> {
  const set = (key: Exclude<keyof Fields, "verdicts" | "rowNotes" | "file">) => (value: string) => setF((was) => ({ ...was, [key]: value }));
  const setIn = (key: "verdicts" | "rowNotes", id: string) => (value: string) => setF((was) => ({ ...was, [key]: { ...was[key], [id]: value } }));
  // Ticks or unticks a briefing's left-out item `n` to file.
  const tick = (n: number) => (e: Event) => {
    const on = (e.target as HTMLInputElement).checked;
    setF((was) => ({ ...was, file: toggled(was.file, n, on) }));
  };
  return { set, setIn, tick };
}

// The items to file `file`, with `n` ticked, once, or unticked.
function toggled(file: number[], n: number, on: boolean): number[] {
  return on ? [...file.filter((m) => m !== n), n] : file.filter((m) => m !== n);
}

// What the form's submit, drop and park send for case `c` from its fields `f`.
function sends(c: Case, f: Fields, preselect: boolean, send: (what: Sends, body: Body) => Promise<void>, refuse: (problem: string) => void) {
  const note = () => f.note.trim();
  function submit(e: Event): void {
    e.preventDefault();
    const { answer, problem } = response(c, f, preselect);
    if (problem) {
      refuse(problem);
      return;
    }
    send("answer", { answer: answer! });
  }
  const drop = () => send("answer", { answer: note() ? { drop: true, note: note() } : { drop: true } });
  const park = () => send("park", note() ? { note: note() } : {});
  return { submit, drop, park };
}

// The note under the fields: the signoff's comment, the reason a briefing
// is sent back with, or the note to the agent, required where the choice
// made needs it.
function noteField(c: Case, f: Fields, set: Form["set"]): ComponentChildren {
  const sendingBack = c.kind === "briefing" && f.briefing === "send_back";
  // The choices that need the note, which the store refuses without one.
  const noteNeeded = sendingBack || (c.kind === "decision" && f.choice === "other") || (c.kind === "signoff" && f.signoff === "changes");
  return html`<label>${c.kind === "signoff" ? "comment" : sendingBack ? "reason: what to fix" : "note to the agent"} <${Hint} keys=${["n"]} /> <${Text} name="note" rows="3" shortcut="n" required=${noteNeeded} value=${f.note} onInput=${set("note")} /></label>`;
}

// The form's send button, labelled acknowledge for an fyi, and its drop,
// or park for a stuck case.
function actions(c: Case, drop: () => void, park: () => void): ComponentChildren {
  return html`<div class="actions">
    <button type="submit" class="primary" id="respond-send" aria-keyshortcuts=${SEND_SHORTCUTS}>${c.kind === "fyi" ? "acknowledge" : "send"} <${Hint} keys=${SEND} /></button>
    ${c.kind === "stuck"
      ? html`<button type="button" id="respond-park" onClick=${park}>park</button>`
      : html`<button type="button" id="respond-drop" onClick=${drop}>drop</button>`}
  </div>`;
}

// The human's withdraw of a case an agent has, answered or picked up, such as
// one whose session died (ST-14), with an optional reason, trimmed as a note
// is. It is folded away: it ends the case for the agent too, whose close is
// then refused. The case view keys it by the state too, so that when a pickup
// lands while it is open it is drawn afresh, folded and empty, as every form
// is on a change of state (IN-06): the pickup shows the agent is still there,
// and the human sees it before withdrawing. A message no agent has picked up
// (`sent`) is taken back the same way, not folded: nothing is lost, and it
// can be sent again. One an agent took says so as a message, which has no
// answer to keep.
function Withdraw({ send, sent = false, message = false }: { send: (what: Sends, body: Body) => Promise<void>; sent?: boolean; message?: boolean }) {
  const [reason, setReason] = useState("");
  function submit(e: Event): void {
    e.preventDefault();
    const why = reason.trim();
    send("withdraw", why ? { reason: why } : {});
  }
  const fields = html`<label>reason <${Text} name="reason" rows="2" value=${reason} onInput=${setReason} /></label>
  <div class="actions"><button type="submit" id="respond-withdraw">withdraw</button></div>`;
  if (sent) {
    return html`<form class="respond withdraw-sent" data-write="withdraw" onSubmit=${submit}><p class="context">no agent has taken it yet. withdraw it to take it back.</p>${fields}</form>`;
  }
  const about = message
    ? "for a message whose agent has gone. an agent still at work on it can no longer close it."
    : "for a case whose agent has gone. an agent still at work on it can no longer close it; the answer stays in the thread.";
  return html`<form class="respond" data-write="withdraw" onSubmit=${submit}><details class="withdraw">
  <summary>withdraw this ${message ? "message" : "case"}</summary>
  <p class="context">${about}</p>
  ${fields}
</details></form>`;
}

// The thread: every event with its author, time and what it said, then any
// rows the fold skipped, such as one from a newer cases (IN-02).
function Thread({ c, now }: { c: Case; now: number }) {
  return html`<section id="thread">
  <h2 class="section">thread</h2>
  <ol class="thread">${thread(c).map(({ event, lines, markdown: text }) => html`<li key=${event.seq} class=${`event author-${event.author}`}>
    <div class="meta"><span class="who">[${event.author}]</span> <span>${event.event}</span>${event.at && html` <span class="age">${stamp(event.at)}, ${old(event.at, now)}</span>`}</div>
    ${lines.map((line) => html`<p>${withCaseLinks(line.text)}</p>${line.previous && "body" in line.previous && html`<details><summary>previous body</summary><${Markdown} text=${line.previous.body} /></details>`}${line.previous && "context" in line.previous && html`<details><summary>previous context</summary><p class="context">${line.previous.context}</p></details>`}`)}
    ${text && html`<${Markdown} text=${text} />`}
  </li>`)}</ol>
  ${(c.problems || []).map((problem) => html`<p class="error">${problem}</p>`)}
</section>`;
}
