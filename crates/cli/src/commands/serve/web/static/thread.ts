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
  let title = "";
  let body = "";
  let context = "";
  let recommend = 0;
  // A briefing's landed and left-out items, as the open and amends set
  // them: an answer files items by their number in the list it saw.
  let landed = false;
  let leftOut: string[] = [];
  // Whether the case stands answered: the agent's next note or amend
  // reopens it. describe.rs reads the same from each event's state before.
  let answered = false;
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
    if (answered && (event.event === "note" || event.event === "amend")) {
      add("reopened the case");
      answered = false;
    }
    if (event.event === "answer") answered = true;
    else if (event.event === "withdraw") answered = false;
    let markdown = "";
    switch (event.event) {
      case "open":
        title = typeof data.title === "string" ? data.title : "";
        body = bodyOf(c, event);
        context = data.context || "";
        recommend = data.recommend || 0;
        landed = !!data.landed;
        leftOut = data.left_out || [];
        // A message says who it went to: its worker, then its labels.
        if (c.kind === "message") {
          const to = [data.worker, ...(data.labels || []).map((label) => `label ${label}`)].filter(Boolean);
          if (to.length) add(`sent to ${to.join(" or ")}`);
        }
        break;
      case "amend": {
        if (data.title && data.title !== title) {
          add(`replaced the title: ${data.title}`);
          title = data.title;
        }
        const replaced = bodyOf(c, event);
        if (replaced && replaced !== body) {
          add("replaced the body", body.trim() ? { body } : null);
          body = replaced;
        }
        for (const option of data.options || []) add(`added option: ${option}`);
        if (data.recommend && data.recommend !== recommend) {
          recommend = data.recommend;
          add(`recommended ${recommend}. ${option(c, recommend) || ""}`);
        }
        for (const row of data.rows || []) add(`added row [${row.id}] ${row.label}`);
        if (typeof data.landed === "boolean" && data.landed !== landed) {
          landed = data.landed;
          add(landed ? "landed" : "not landed");
        }
        const items = data.left_out || [];
        if (items.length && !sameList(items, leftOut)) {
          leftOut = items;
          add(`replaced the left-out items: ${items.join("; ")}`);
        }
        for (const link of data.drop_links || []) add(`dropped link: ${link}`);
        for (const link of data.links || []) add(`added link: ${link}`);
        for (const label of data.labels || []) add(`added label: ${label}`);
        if (data.context && data.context !== context) {
          add(`replaced the context: ${data.context}`, context.trim() ? { context } : null);
          context = data.context;
        }
        break;
      }
      case "answer":
        answer(c, data, add, leftOut);
        break;
      case "pickup":
        if (data.by) add(`by ${data.by}`);
        break;
      case "note":
        markdown = data.body || "";
        break;
      case "close":
        markdown = data.outcome || "";
        for (const link of data.links || []) add(link);
        break;
      case "withdraw":
        if (data.reason) add(`reason: ${data.reason}`);
        break;
      case "park":
        if (data.note) add(`note: ${data.note}`);
        break;
    }
    return { event, lines, markdown };
  });
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
  const chosen = option(c, data.choice);
  if (chosen !== undefined) add(`chose ${data.choice}. ${chosen}`);
  else if (data.other) add("chose other");
  else if (data.signoff) add(`signoff: ${data.signoff}`);
  else if (data.briefing) add(`briefing: ${data.briefing.replace(/_/g, " ")}`);
  else if (data.text && c.kind === "question") add(`reply: ${data.text}`);
  else if (data.text) add(`guidance: ${data.text}`);
  else if (data.drop) add("drop");
  else if (data.ack) add("acknowledged");
  for (const row of data.rows || []) {
    // `reject` is kept as it is, and shown to people as "don't run" (HU-01).
    const verdict = row.verdict === "reject" ? "don't run" : row.verdict;
    add(row.note ? `[${row.id}] ${verdict}: ${row.note}` : `[${row.id}] ${verdict}`);
  }
  if (data.note) add(`note: ${data.note}`);
  for (const n of data.file || []) add(`file ${n}. ${leftOut[n - 1] || ""}`);
}
