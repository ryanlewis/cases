// Whether a case is a follow-up: one the agent reopened with a note after it
// was answered (AG-09), so that the page and the list can say what is new
// since the answer rather than show the case as it was first asked. A case
// the agent reopened with an amend is not one: the amend asks the question
// afresh, and the old answer stays in the thread.
import type { Case, Event } from "./types.js";

// A follow-up: the answer it reopened, and the agent's notes since, oldest
// first.
export interface FollowUp {
  answer: Event;
  notes: Event[];
}

// The follow-up case `c` is, or null. It is one when it is open, it has been
// answered, no amend reopened it after its last answer (an amend whose `from`
// is answered or picked up), and the last event the agent wrote after that
// answer is a note. An amend after the note that changes the question (the
// case's `amend_seq`) asks it afresh, so the case is drawn as it was first
// asked; one that only adds a label, a link or context does not.
export function followUp(c: Case): FollowUp | null {
  if (c.state !== "open") return null;
  const events = c.events || [];
  let last = -1;
  events.forEach((e, i) => {
    if (e.event === "answer") last = i;
  });
  const answer = events[last];
  if (!answer) return null;
  const after = events.slice(last + 1);
  if (after.some((e) => e.event === "amend" && (e.from === "answered" || e.from === "pickedup"))) return null;
  const agent = after.filter((e) => e.author === "agent" && e.event !== "amend");
  const lastNote = agent[agent.length - 1];
  if (lastNote?.event !== "note" || (c.amend_seq || 0) > lastNote.seq) return null;
  return { answer, notes: agent.filter((e) => e.event === "note") };
}
