// Whether a case is still the one a form was drawn from: the core's
// freshness rule (`Case::since`, spec/vectors/README.md, "Freshness"),
// which test/freshness.json pins with the core's vectors. The case view's
// forms and a row's withdraw follow it (IN-05, IN-06).
import type { Case, State } from "./types.js";

// Where a form was drawn: the case's revision, and the highest seq among
// its rows, skipped ones included. A new row is numbered above every row the
// case has, so the events after that seq are the ones since.
export interface Mark {
  revision: number;
  seq: number;
}

// The mark of a form drawn from case `c` as it is now.
export function mark(c: Case): Mark {
  return { revision: c.revision, seq: c.last_seq };
}

// How case `c` changed since a form was drawn from it at `drawn`.
// "unchanged": nothing, or only notes and amends that kept the case in its
// state and left the question as it was. "moved": the case moved to other
// states, each passed once, by rows that all folded, with the question as it
// was. "changed": anything else, such as a row the fold skipped, a state
// passed twice, or an amend that changed the question (IN-05, IN-06). A
// message's amends are the human's own edits (IN-13), so they change no
// question.
export type Since = "unchanged" | "moved" | "changed";

export function since(c: Case, drawn: Mark): Since {
  if (drawn.revision === c.revision) return "unchanged";
  if (drawn.revision > c.revision) return "changed";
  // The events since: seqs rise, as the store numbers its rows.
  const after = (c.events || []).filter((e) => e.seq > drawn.seq);
  // Fewer than the rows added: the fold skipped one. More: the seq is below
  // the one the revision had.
  if (after.length !== c.revision - drawn.revision) return "changed";
  if ((c.amend_seq || 0) > drawn.seq && c.kind !== "message") return "changed";
  if (after.every((e) => (e.event === "note" || e.event === "amend") && e.from === c.state)) return "unchanged";
  // The states the case passed: each event's state before it, then its
  // state now. The open has none: the form was drawn before the case had a
  // state.
  const left = new Set<State>();
  let at: State | undefined;
  for (const next of [...after.map((e) => e.from), c.state]) {
    if (!next) return "changed";
    if (next === at) continue;
    if (left.has(next)) return "changed";
    if (at) left.add(at);
    at = next;
  }
  return "moved";
}

// The mark a form drawn at `was` carries once the case, still in the state
// the form was drawn in, is `c`: the case's now when it is unchanged since,
// such as after a note or the human's own edit of a message, and `was`
// otherwise, so that the form's send is refused as stale (IN-05).
export function followed(was: Mark, c: Case): Mark {
  return since(c, was) === "unchanged" ? mark(c) : was;
}
