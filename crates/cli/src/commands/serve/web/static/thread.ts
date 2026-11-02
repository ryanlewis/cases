// What each event of a case said, as lines for its thread (IN-02), read
// from each event's record as stored, `events[].data`. The CLI's `show`
// says the same in src/describe.rs.
import type { Actor, Case, Event, EventData } from "./types.js";

// A line of an event: its text, and on the line that says an amend replaced
// the body or context, the text the case had before.
export interface Line {
  text: string;
  previous: { body: string } | { context: string } | null;
}

// An event with the lines it says, and a note's body or a close's outcome.
export interface Said {
  event: Event;
  lines: Line[];
  markdown: string;
}

// Adds a line, or the lines of a text.
type Add = (text: string, previous?: Line["previous"]) => void;

// The option of case `c` numbered `n` from 1, as a choice or a recommend
// names it, if it has one. describe.rs has the same.
export function option(c: Case, n: number | undefined): string | undefined {
  return n && n > 0 ? (c.options || [])[n - 1] : undefined;
}

// Who an actor is, for people: the name its row gives, its id when the row
// has no name, and "deleted user" when the row is gone (AG-13, IN-02).
export function who(actor: Actor | undefined): string {
  if (!actor) return "";
  if (actor.name) return actor.name;
  if (actor.kind) return actor.id;
  return "deleted user";
}

// The body an open or amend of case `c` set: its record's, or when the
// record leaves it out as the case's own (`body_in_case`), the case's.
function bodyOf(c: Case, event: Event): string {
  return event.body_in_case ? c.body || "" : event.data?.body || "";
}

// Each event of case `c`, in order, with its lines: `{ event, lines,
// markdown }`. A line is `{ text, previous }`, where `previous`, on the line
// that says an amend replaced the body or context, is `{ body }` or
// `{ context }` with the text the case had before, unless that was blank.
// `markdown` is a note's body or a close's outcome.
export function thread(c: Case): Said[] {
  const seen: Seen = { title: "", body: "", context: "", recommend: 0, landed: false, leftOut: [], answered: false };
  return (c.events || []).map((event): Said => {
    const data = event.data || {};
    const lines: Line[] = [];
    const add: Add = (text, previous = null) => {
      const parts = String(text).trim().split("\n");
      parts.forEach((part, i) => lines.push({ text: part, previous: i === parts.length - 1 ? previous : null }));
    };
    const name = who(event.actor);
    // A pickup whose `by` names its actor says so once.
    if (name && !(event.event === "pickup" && data.by === name)) add(`by ${name}`);
    reopened(event, seen, add);
    const markdown = say[event.event]?.(c, event, data, seen, add) || "";
    return { event, lines, markdown };
  });
}

// What the events before the one being said had set: the open's and amends'
// title, body, context, recommended option, and a briefing's landed and
// left-out items, which an answer files by their number in the list it saw.
// `answered` is whether the case stands answered: the agent's next note or
// amend reopens it. describe.rs reads the same from each event's state
// before.
interface Seen {
  title: string;
  body: string;
  context: string;
  recommend: number;
  landed: boolean;
  leftOut: string[];
  answered: boolean;
}

// Says a note or amend reopened an answered case, and keeps whether the
// case stands answered after `event`.
function reopened(event: Event, seen: Seen, add: Add): void {
  if (seen.answered && (event.event === "note" || event.event === "amend")) {
    add("reopened the case");
    seen.answered = false;
  }
  if (event.event === "answer") seen.answered = true;
  else if (event.event === "withdraw") seen.answered = false;
}

// Adds what each event of case `c`, with record `data`, said, and gives
// its markdown: a note's body or a close's outcome. An event left out says
// nothing of its own.
type Say = (c: Case, event: Event, data: EventData, seen: Seen, add: Add) => string | void;
const say: Partial<Record<Event["event"], Say>> = {
  open: opened,
  amend: amended,
  answer: (c, _event, data, seen, add) => answer(c, data, add, seen.leftOut),
  pickup: (_c, _event, data, _seen, add) => {
    if (data.by) add(`by ${data.by}`);
  },
  note: (_c, _event, data) => data.body || "",
  close: (_c, _event, data, _seen, add) => {
    for (const link of data.links || []) add(link);
    return data.outcome || "";
  },
  withdraw: (_c, _event, data, _seen, add) => {
    if (data.reason) add(`reason: ${data.reason}`);
  },
  park: (_c, _event, data, _seen, add) => {
    if (data.note) add(`note: ${data.note}`);
  },
};

// An open: what it set, and for a message who it went to, its worker then
// its labels.
function opened(c: Case, event: Event, data: EventData, seen: Seen, add: Add): void {
  seen.title = typeof data.title === "string" ? data.title : "";
  seen.body = bodyOf(c, event);
  seen.context = data.context || "";
  seen.recommend = data.recommend || 0;
  seen.landed = !!data.landed;
  seen.leftOut = data.left_out || [];
  if (c.kind === "message") {
    const to = [data.worker, ...(data.labels || []).map((label) => `label ${label}`)].filter(Boolean);
    if (to.length) add(`sent to ${to.join(" or ")}`);
  }
}

// An amend: each change it made, in order, against what was set before.
function amended(c: Case, event: Event, data: EventData, seen: Seen, add: Add): void {
  amendedText(c, event, data, seen, add);
  amendedChoices(c, data, seen, add);
  amendedBriefing(data, seen, add);
  for (const link of data.drop_links || []) add(`dropped link: ${link}`);
  for (const link of data.links || []) add(`added link: ${link}`);
  for (const label of data.labels || []) add(`added label: ${label}`);
  if (data.context && data.context !== seen.context) {
    add(`replaced the context: ${data.context}`, seen.context.trim() ? { context: seen.context } : null);
    seen.context = data.context;
  }
}

// An amend's replaced title and body, the body's line with the one before.
function amendedText(c: Case, event: Event, data: EventData, seen: Seen, add: Add): void {
  if (data.title && data.title !== seen.title) {
    add(`replaced the title: ${data.title}`);
    seen.title = data.title;
  }
  const replaced = bodyOf(c, event);
  if (replaced && replaced !== seen.body) {
    add("replaced the body", seen.body.trim() ? { body: seen.body } : null);
    seen.body = replaced;
  }
}

// An amend's added options, recommended option and added approval rows.
function amendedChoices(c: Case, data: EventData, seen: Seen, add: Add): void {
  for (const option of data.options || []) add(`added option: ${option}`);
  if (data.recommend && data.recommend !== seen.recommend) {
    seen.recommend = data.recommend;
    add(`recommended ${seen.recommend}. ${option(c, seen.recommend) || ""}`);
  }
  for (const row of data.rows || []) add(`added row [${row.id}] ${row.label}`);
}

// An amend of a briefing: whether it landed, and its left-out items.
function amendedBriefing(data: EventData, seen: Seen, add: Add): void {
  if (typeof data.landed === "boolean" && data.landed !== seen.landed) {
    seen.landed = data.landed;
    add(seen.landed ? "landed" : "not landed");
  }
  const items = data.left_out || [];
  if (items.length && !sameList(items, seen.leftOut)) {
    seen.leftOut = items;
    add(`replaced the left-out items: ${items.join("; ")}`);
  }
}

// Whether lists `a` and `b` hold the same items in the same order.
function sameList(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((item, i) => item === b[i]);
}

// A briefing's left-out items as case `c`'s open and amends had set them
// before the event `seq`: the list an answer at `seq` numbered.
function leftOutBefore(c: Case, seq: number): string[] {
  let items: string[] = [];
  for (const e of c.events || []) {
    if (e.seq >= seq) break;
    if (e.event === "open") items = e.data.left_out || [];
    else if (e.event === "amend" && (e.data.left_out || []).length) items = e.data.left_out!;
  }
  return items;
}

// What answer `data` of case `c` said, as lines of text: the same lines the
// thread gives it, less who answered. A briefing's filed items are named
// from the left-out items the answer saw, as the thread names them.
export function answered(c: Case, data: EventData): string[] {
  const lines: string[] = [];
  const event = (c.events || []).find((e) => e.data === data);
  const items = event ? leftOutBefore(c, event.seq) : c.left_out || [];
  answer(c, data, (text) => lines.push(...String(text).trim().split("\n")), items);
  return lines;
}

// What an answer said. A briefing's answer files its left-out items by
// number in `leftOut`, the list it saw; describe.rs says the same.
function answer(c: Case, data: EventData, add: Add, leftOut: string[]): void {
  const said = form(c, data);
  if (said) add(said);
  for (const row of data.rows || []) {
    // `reject` is kept as it is, and shown to people as "don't run" (HU-01).
    const verdict = row.verdict === "reject" ? "don't run" : row.verdict;
    add(row.note ? `[${row.id}] ${verdict}: ${row.note}` : `[${row.id}] ${verdict}`);
  }
  if (data.note) add(`note: ${data.note}`);
  for (const n of data.file || []) add(`file ${n}. ${leftOut[n - 1] || ""}`);
}

// The first of an answer's forms it has, as its line, if any.
function form(c: Case, data: EventData): string | undefined {
  const chosen = option(c, data.choice);
  if (chosen !== undefined) return `chose ${data.choice}. ${chosen}`;
  if (data.other) return "chose other";
  if (data.signoff) return `signoff: ${data.signoff}`;
  if (data.briefing) return `briefing: ${data.briefing.replace(/_/g, " ")}`;
  if (data.text) return c.kind === "question" ? `reply: ${data.text}` : `guidance: ${data.text}`;
  if (data.drop) return "drop";
  if (data.ack) return "acknowledged";
  return undefined;
}
