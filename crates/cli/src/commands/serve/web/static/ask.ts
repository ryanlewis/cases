// What an inbox card says is asked, and the groups the inbox is drawn in.
// Plain functions of a case; web/test/ask.test.js runs them in node.
import type { FollowUp } from "./followup.js";
import type { Case } from "./types.js";

// The inbox's groups, in the order they are drawn.
export type Group = "blocking" | "today" | "whenever" | "parked";

export const GROUPS: { group: Group; label: string }[] = [
  { group: "blocking", label: "Blocking" },
  { group: "today", label: "Today" },
  { group: "whenever", label: "Whenever" },
  { group: "parked", label: "Parked" },
];

// The group case `c` is drawn in: parked, whatever its urgency, since
// nobody waits on it until it is resumed; otherwise its urgency.
export function group(c: Case): Group {
  return c.state === "parked" ? "parked" : c.urgency;
}

// The cases in their groups, in the order above, each in the order it is
// given; a group with no case is left out.
export function groups(cases: Case[]): { group: Group; label: string; cases: Case[] }[] {
  return GROUPS.map((g) => ({ ...g, cases: cases.filter((c) => group(c) === g.group) })).filter((g) => g.cases.length);
}

// The cases in the order the grouped inbox draws them.
export function ordered(cases: Case[]): Case[] {
  return groups(cases).flatMap((g) => g.cases);
}

// A line without its markdown emphasis and code marks. A lone underscore
// stays, since it is more often part of a name than emphasis.
function plain(line: string): string {
  return line.replace(/\*+|__|`/g, "").trim();
}

// A body's task list: how many of its `- [ ]` items are ticked, or null
// when it has none.
export function checks(body: string | undefined): { done: number; total: number } | null {
  let done = 0;
  let total = 0;
  for (const line of (body || "").split("\n")) {
    const m = /^\s*[-*+]\s+\[([ xX])\]\s/.exec(line);
    if (!m) continue;
    total++;
    if (m[1] !== " ") done++;
  }
  return total ? { done, total } : null;
}

// The first line of a body that is not blank or a code fence, as plain
// text: without the marks that start a heading, quote or list item, or
// those of emphasis.
export function firstLine(body: string | undefined): string {
  for (const line of (body || "").split("\n")) {
    if (/^\s*(?:```|~~~)/.test(line)) continue;
    const t = plain(line.replace(/^\s*(?:#{1,6}\s+|>\s*|[-*+]\s+(?:\[[ xX]\]\s+)?|\d+[.)]\s+)/, ""));
    if (t) return t;
  }
  return "";
}

// `n` things, as "1 option" or "4 options".
function count(n: number, one: string, many: string): string {
  return `${n} ${n === 1 ? one : many}`;
}

// What case `c` asks, in one line, for its card. `f` is the case as a
// follow-up, or null when it is not one.
//
//   parked     parked: the park note
//   follow-up  follow-up · you chose 2 (or chose other, or answered)
//   decision   4 options · recommends 1 (when the agent recommends one)
//   approval   3 scripts to approve (items, when a row has no script)
//   signoff    4 of 5 checks (from the body's task list), or accept or
//              request changes when it has none
//   briefing   waiting to land (or landed) · 2 left out (when it left some
//              out)
//   others     the body's first line, or the context
export function ask(c: Case, f: FollowUp | null): string {
  if (c.state === "parked") {
    const note = c.park && c.park.note && c.park.note.trim();
    return note ? `parked: ${note}` : "parked";
  }
  if (f) {
    const a = f.answer.data || {};
    if (a.other) return "follow-up · you chose other";
    if (typeof a.choice === "number" && a.choice > 0) return `follow-up · you chose ${a.choice}`;
    return "follow-up · you answered";
  }
  switch (c.kind) {
    case "decision": {
      const options = count((c.options || []).length, "option", "options");
      return c.recommend ? `${options} · recommends ${c.recommend}` : options;
    }
    case "approval": {
      const rows = c.rows || [];
      const scripts = rows.every((r) => r.script);
      return `${scripts ? count(rows.length, "script", "scripts") : count(rows.length, "item", "items")} to approve`;
    }
    case "signoff": {
      const k = checks(c.body);
      return k ? `${k.done} of ${count(k.total, "check", "checks")}` : "accept or request changes";
    }
    case "briefing": {
      const n = (c.left_out || []).length;
      const mark = c.landed ? "landed" : "waiting to land";
      return n ? `${mark} · ${n} left out` : mark;
    }
    default:
      return firstLine(c.body) || plain(c.context || "");
  }
}
