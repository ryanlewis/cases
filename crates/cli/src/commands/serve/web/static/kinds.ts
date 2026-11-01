// The slot for a glyph of a case's kind, on each card and in the case's
// properties. It is empty: a look draws the glyph into it from its own
// images or characters in CSS, or leaves it hidden. The kind is said in
// words beside it, so the glyph is hidden from screen readers.
import { html } from "./html.js";
import type { Kind } from "./types.js";

export function KindIcon({ kind }: { kind: Kind }) {
  return html`<span class=${`kind-icon kind-${kind}`} aria-hidden="true"></span>`;
}
