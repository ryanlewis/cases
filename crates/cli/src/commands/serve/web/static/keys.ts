// Keyboard shortcuts (IN-09).
//
// Cmd+Enter or Ctrl+Enter sends an open case's answer, from any of its
// fields: the same send as the button, so the browser checks the required
// choices first, and the case view sends it once.
//
// Other keys do nothing while a text field has the focus, a dialog is open,
// or Cmd, Ctrl or Alt is held:
//
//   1-9  choose that option, where the form has one set of choices
//        (decision, sign-off, stuck)
//   1-3  on an approval, approve, hold or don't run the current row: the
//        row with the focus, else the first with no verdict. The focus then
//        goes to the next row with no verdict, or stays on this one. A held
//        key counts once. A verdict the options only pre-selected counts as
//        none, so with every row pre-selected the keys walk down the rows
//   n    go to the note to the agent (the comment, on a sign-off)
//   j k  open the next or previous case in the list, under its filter
//   ?    show these keys, in the dialog `keys()` gives
//
// Listening on the document covers every case the page draws.
//
// The keys are also shown where they work, as glyphs (`Hint`): a number on
// each choice (on every row of an approval), n by the note, the send keys on the send button, and j, k and ? by the
// list. The key hints option shows or hides them (theme.ts); the keys dialog
// and the properties' keys stay either way.
import { html } from "./html.js";

// Whether this is a Mac, whose send key the hints draw as ⌘ rather than
// Ctrl. Both send on every system.
const mac = typeof navigator !== "undefined" && /Mac|iPhone|iPad|iPod/.test(navigator.platform);

// The send keys, as the hints draw them.
export const SEND = [mac ? "⌘" : "Ctrl", "↵"];

// The send keys, for aria-keyshortcuts.
export const SEND_SHORTCUTS = "Meta+Enter Control+Enter";

// Glyphs for `keys`, then `label` if given. On a control, which has its
// own label, they are hidden from screen readers: its aria-keyshortcuts
// says the keys.
export function Hint({ keys, label }: { keys: string[]; label?: string }) {
  const glyphs = keys.map((k) => html`<kbd>${k}</kbd>`);
  return label
    ? html`<span class="key-hint">${glyphs} ${label}</span>`
    : html`<span class="key-hint" aria-hidden="true">${glyphs}</span>`;
}

// Whether `el` takes typed text.
function typing(el: HTMLElement | null): boolean {
  if (!el || !el.tagName) return false;
  if (el.isContentEditable || el.tagName === "TEXTAREA" || el.tagName === "SELECT") return true;
  return el.tagName === "INPUT" && !["radio", "checkbox", "button", "submit", "reset"].includes((el as HTMLInputElement).type);
}

// The open case's answer form, if the page shows one. On a narrow window
// the inbox hides its case, form and all, and the keys leave it alone.
function answerForm(): HTMLFormElement | null {
  const form = document.querySelector('#case form.respond[data-write="answer"]') as HTMLFormElement | null;
  return form && form.getClientRects().length ? form : null;
}

// The radios of the form's one set of choices, in page order.
function choices(form: HTMLFormElement): HTMLInputElement[] {
  return Array.from(form.querySelectorAll<HTMLInputElement>('input[type="radio"]'));
}

// The approval row the number keys set, of rows whose verdicts `done` says:
// the row with the focus, `at` (-1 for none), else the first with no
// verdict. -1 when there is no such row.
export function rowFor(done: boolean[], at: number): number {
  return at >= 0 ? at : done.indexOf(false);
}

// The row the focus goes to once row `at` has its verdict: the next after
// it with no verdict, else row `at` itself.
export function rowAfter(done: boolean[], at: number): number {
  const next = done.indexOf(false, at + 1);
  return next < 0 ? at : next;
}

// Gives the current row of an approval's form its `n`th verdict, and moves
// the focus on. Whether there was such a verdict to give.
function verdict(form: HTMLFormElement, n: number): boolean {
  const rows = Array.from(form.querySelectorAll<HTMLFieldSetElement>("fieldset.row"));
  const radios = (row: HTMLFieldSetElement) => row.querySelectorAll<HTMLInputElement>('input[type="radio"]');
  // A pre-selected verdict (case.js) is not one chosen.
  const done = rows.map((row) => !("preset" in row.dataset) && Array.from(radios(row)).some((r) => r.checked));
  const at = rowFor(done, rows.findIndex((row) => row.contains(document.activeElement)));
  const radio = at >= 0 ? radios(rows[at]!)[n - 1] : undefined;
  if (!radio) return false;
  // As a click would, so the form hears the choice.
  radio.click();
  done[at] = true;
  const next = rowAfter(done, at);
  (next === at ? radio : radios(rows[next]!)[0]!).focus();
  return true;
}

// Opens the case `by` cards after the one shown, from the list.
function step(by: number): void {
  // A card in a folded group (the briefings answered, picked up or not) is
  // not one to move to, as the page's list leaves it out (lists.ts).
  const cards = Array.from(document.querySelectorAll("#inbox a.card") as NodeListOf<HTMLAnchorElement>).filter((a) => !a.closest("details:not([open])"));
  if (!cards.length) return;
  const at = cards.findIndex((a) => a.getAttribute("aria-current") === "page");
  const next = at < 0 ? (by > 0 ? 0 : cards.length - 1) : at + by;
  if (next >= 0 && next < cards.length && next !== at) cards[next]!.click();
}

export function keys(dialog: () => HTMLDialogElement | null): void {
  document.addEventListener("keydown", (e) => {
    if (e.defaultPrevented || e.isComposing) return;
    const open = document.querySelector("dialog[open]");

    if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey) {
      const form = answerForm();
      if (!form || open || e.repeat) return;
      // A field in some other form keeps the key to itself, and so does a
      // link, which the browser opens in a new tab on Cmd or Ctrl+Enter.
      if ((e.target as HTMLInputElement).form && (e.target as HTMLInputElement).form !== form) return;
      if ((e.target as Element).closest && (e.target as Element).closest("a[href]")) return;
      e.preventDefault();
      form.requestSubmit(form.querySelector("#respond-send"));
      return;
    }

    if (e.metaKey || e.ctrlKey || e.altKey || typing(e.target as HTMLElement | null) || open) return;

    if (e.key === "?") {
      const keys = dialog();
      if (keys && keys.showModal) {
        e.preventDefault();
        keys.showModal();
      }
      return;
    }
    if (e.key === "n") {
      const form = answerForm();
      const note = form && form.querySelector<HTMLTextAreaElement>('textarea[name="note"]');
      if (!note) return;
      // Kept from the field, which takes the focus before the key is typed.
      e.preventDefault();
      note.focus();
      return;
    }
    if (e.key === "j" || e.key === "k") {
      e.preventDefault();
      step(e.key === "j" ? 1 : -1);
      return;
    }
    if (/^[1-9]$/.test(e.key)) {
      const form = answerForm();
      // An approval's rows have a set of choices each.
      // A held key repeats, and each repeat would give the next row its
      // verdict too, so only the first press counts.
      if (form && form.querySelector("fieldset.row")) {
        if (!e.repeat && verdict(form, Number(e.key))) e.preventDefault();
        return;
      }
      const radio = form && choices(form)[Number(e.key) - 1];
      if (!radio) return;
      e.preventDefault();
      // As a click would, so the form hears the choice.
      radio.click();
      radio.focus();
    }
  });
}
