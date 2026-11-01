// A case's properties (IN-02): what it is, the option the agent recommends,
// who it is from and for, when it was opened and changed, its id, and the
// keys that work on it. Its links are under its body (case.js).
// One list, which the stylesheet draws as a column at the right of the case
// when its pane is wide, and as a row of chips under its title when not.
// A property the case does not have is left out.
import { html } from "./html.js";
import { slug } from "./ids.js";
import { KindIcon } from "./kinds.js";
import { option, who } from "./thread.js";
import { ago, stamp } from "./time.js";
import type { ComponentChildren } from "preact";
import type { Case, Time } from "./types.js";

// A property: its name, what it is, and whether the chip row names it too.
// The chip row leaves out a name the value says by itself, such as
// "decision" or "blocking".
interface Prop {
  name: string;
  value: ComponentChildren;
  named: boolean;
}

// When `at` was: its age, with the time itself beside it in the column.
function When({ at, now }: { at: Time; now: number }) {
  return html`<time datetime=${at} title=${stamp(at)}>${ago(at, now)}</time><span class="sub">${stamp(at)}</span>`;
}

// The keys that work on case `c` (IN-09, keys.js): the number keys choose
// where its open form has one set of choices, or give an approval's row its
// verdict, n goes to its note, and the send keys send it.
function keys(c: Case): [string, ComponentChildren][] {
  const out: [string, ComponentChildren][] = [];
  if (c.state === "open") {
    const choices = c.kind === "decision" ? (c.options || []).length + 1 : c.kind === "signoff" || c.kind === "stuck" || c.kind === "briefing" ? 2 : 0;
    if (choices > 0) out.push(["choose", html`<kbd>1</kbd>–<kbd>${Math.min(choices, 9)}</kbd>`]);
    if (c.kind === "approval") out.push(["verdict", html`<kbd>1</kbd> <kbd>2</kbd> <kbd>3</kbd>`]);
    out.push(["note", html`<kbd>n</kbd>`]);
    out.push(["send", html`<kbd>⌘</kbd>/<kbd>Ctrl</kbd> <kbd>Enter</kbd>`]);
  }
  out.push(["next, previous", html`<kbd>j</kbd> <kbd>k</kbd>`]);
  out.push(["all keys", html`<kbd>?</kbd>`]);
  return out;
}

export function Properties({ c, now }: { c: Case; now: number }) {
  const props: Prop[] = [
    { name: "kind", value: html`<${KindIcon} kind=${c.kind} />${c.kind}`, named: false },
    { name: "urgency", value: html`<span class="badge urgency">${c.urgency}</span>`, named: false },
    { name: "state", value: html`<span class=${c.state === "parked" ? "badge state parked" : "badge state"}>${c.state}</span>`, named: false },
  ];
  if (c.recommend) {
    // The option's number and text, so a case whose form is gone still says
    // which; cut to one line, with all of it in the title.
    const text = `${c.recommend}. ${option(c, c.recommend) || ""}`;
    props.push({ name: "recommends", value: html`<span class="recommends" title=${text}>${text}</span>`, named: true });
  }
  // A message's worker is the one it is sent to.
  if (c.worker) props.push({ name: c.kind === "message" ? "to" : "worker", value: c.worker, named: c.kind === "message" });
  if (c.for) props.push({ name: "for", value: c.for, named: true });
  // Who sent a message: its page has no thread to say so.
  if (c.kind === "message" && who(c.actor)) props.push({ name: "from", value: who(c.actor), named: true });
  if ((c.labels || []).length > 0) {
    props.push({ name: "labels", value: c.labels!.map((label) => html`<span class="tag">${label}</span>`), named: false });
  }
  if (c.opened_at) props.push({ name: "opened", value: html`<${When} at=${c.opened_at} now=${now} />`, named: true });
  if (c.updated_at && c.updated_at !== c.opened_at) {
    props.push({ name: "updated", value: html`<${When} at=${c.updated_at} now=${now} />`, named: true });
  }
  const amends = (c.events || []).filter((e) => e.event === "amend");
  const amend = amends[amends.length - 1];
  if (amend) {
    props.push({ name: "amended", value: amend.at ? html`<${When} at=${amend.at} now=${now} />` : "yes", named: true });
  }
  // The id as agents write a case in chat, case:ID, so the human can find
  // it here; the slug form, as the address has it, in its title.
  props.push({ name: "id", value: html`<code title=${`${c.id}-${slug(c.title)}`}>case:${c.id}</code>`, named: false });
  return html`<aside class=${`props urgency-${c.urgency}`} aria-label="properties">
  <h2 class="section">properties</h2>
  <dl>${props.map(({ name, value, named }) => html`<div class=${`prop-${name}${named ? " named" : ""}`}><dt>${name}</dt><dd>${value}</dd></div>`)}</dl>
  <div class="props-keys"><h2 class="section">keys</h2><dl>${keys(c).map(([name, value]) => html`<div><dt>${name}</dt><dd>${value}</dd></div>`)}</dl></div>
</aside>`;
}
