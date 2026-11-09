// A decision answered "other" with a note, picked up, then reopened by the
// agent's follow-up note: the case of the report that led to follow-up
// first, as serve's API gives it after each step.
const at = (minute) => `2026-09-25T13:${String(minute).padStart(2, "0")}:00Z`;
const agent = { id: "a1", kind: "agent", name: "gift-agent" };
const human = { id: "h1", kind: "human", name: "ryan" };

export const body = "Which way for the anniversary present?";
export const options = ["a watch", "a weekend away", "a print"];

const open = { seq: 1, author: "agent", event: "open", file: "", at: at(10), actor: agent, data: { body, options } };
const answer = { seq: 2, author: "human", event: "answer", from: "open", file: "", at: at(20), actor: human, data: { other: true, note: "option 2 is out" } };
const pickup = { seq: 3, author: "agent", event: "pickup", from: "answered", file: "", at: at(21), actor: agent, data: { by: "gift-agent" } };
const note = { seq: 4, author: "agent", event: "note", from: "pickedup", file: "", at: at(30), actor: agent, data: { body: "Follow-up: a watch or a print, then?" } };

// The case after `events`, in `state`.
export function decision(state, ...events) {
  return {
    id: "aB3xQ",
    state,
    kind: "decision",
    urgency: "today",
    title: "Which way for the anniversary present?",
    body,
    options,
    opened_at: at(10),
    events,
    revision: events.length,
    amend_seq: 0,
    last_seq: Math.max(0, ...events.map((e) => e.seq)),
  };
}

export const steps = { open, answer, pickup, note };

// A form's fields, as the answer form keeps them, with `extra` set.
export const fields = (extra = {}) => ({ choice: "", verdicts: {}, rowNotes: {}, signoff: "", briefing: "", file: [], stuck: "", text: "", note: "", ...extra });
