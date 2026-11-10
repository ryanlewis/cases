// The options dialog (IN-10), the list of keys (IN-09), and the desktop
// notifications the options turn on (IN-08). The choices themselves are kept
// and applied by theme.js, as window.casesPrefs.
import { useState } from "preact/hooks";
import * as api from "./api.js";
import { html } from "./html.js";
import type { ComponentChildren, Ref } from "preact";
import type { Notification as Item } from "./types.js";

// The last notification shown, and the boot of serve it is from.
interface Cursor {
  boot: string;
  id: number;
}

const prefs = window.casesPrefs;

// Whether this browser may show notifications: granted, denied, default
// (it asks first) or unsupported.
function permission() {
  return "Notification" in window ? Notification.permission : "unsupported";
}

const STATUS = {
  granted: "allowed in this browser.",
  denied: "blocked in this browser. allow them in its site settings for this address.",
  default: "the browser asks before the first one.",
  unsupported: "this browser cannot show them.",
};

// Closes `dialog` on a click on its backdrop: a press and release on the
// dialog itself, whose form fills it, so that a drag that starts inside the
// form and ends outside does not close it. Escape and the close button
// (a method=dialog form) close it by themselves.
function backdrop() {
  let down = false;
  return {
    onPointerDown: (e: PointerEvent) => (down = e.target === e.currentTarget),
    onClick: (e: MouseEvent) => {
      if (e.target === e.currentTarget && down) (e.currentTarget as HTMLDialogElement).close();
      down = false;
    },
  };
}

interface Group {
  name: string;
  legend: string;
  choices: [string, ComponentChildren][];
}

// The groups in two columns, appearance then behaviour, side by side when the
// dialog is wide and one after the other when it is not, so the order read,
// and tabbed through, is the order drawn. A group with one choice is not
// shown.
const COLUMNS: { heading: string; groups: Group[] }[] = [
  {
    heading: "appearance",
    groups: [
      { name: "look", legend: "look", choices: [["mono", "mono"], ["daylight", "daylight"], ["keel", "keel"]] },
      { name: "theme", legend: "theme", choices: [["system", "system"], ["light", "light"], ["dark", "dark"]] },
      {
        name: "face",
        legend: "prose face",
        choices: [["mono", "mono"], ["sans", html`<span class="face-sans">sans</span>`], ["serif", html`<span class="face-serif">serif</span>`]],
      },
      { name: "size", legend: "text size", choices: [["small", "small"], ["medium", "medium"], ["large", "large"]] },
      { name: "wrap", legend: "long lines in code", choices: [["scroll", "scroll sideways"], ["wrap", "wrap"]] },
    ],
  },
  {
    heading: "behaviour",
    groups: [
      { name: "links", legend: "links to other sites", choices: [["new", "open in a new tab"], ["same", "open in this tab"]] },
      { name: "notify", legend: "desktop notifications", choices: [["off", "off"], ["blocking", "blocking cases"], ["all", "every case"]] },
      { name: "hints", legend: "key hints", choices: [["show", "show"], ["hide", "hide"]] },
      { name: "approve", legend: "pre-select approve for approvals", choices: [["off", "off"], ["on", "on"]] },
    ],
  },
];

// The options, kept in this browser and applied at once, in this tab and
// the others. `dialog` is the ref the header's options chip opens.
export function Options({ dialog }: { dialog: Ref<HTMLDialogElement> }) {
  const [, redraw] = useState(0);
  const refresh = () => redraw((n) => n + 1);
  // The browser asks only from a click: the allow button, or choosing to
  // turn notifications on. A refusal turns them back off.
  const ask = () =>
    Notification.requestPermission().then((answer) => {
      if (answer !== "granted") prefs.write("notify", "off");
      prefs.apply();
      refresh();
    });
  const choose = (name: string, value: string) => {
    prefs.write(name, value);
    prefs.apply();
    refresh();
    if (name === "notify" && value !== "off" && permission() === "default") ask();
  };
  const reset = () => {
    for (const option of prefs.options) prefs.write(option.name, option.values[0]!);
    prefs.apply();
    refresh();
  };
  const allowed = permission();
  const shown = COLUMNS.map((column) => ({ ...column, groups: column.groups.filter((group) => group.choices.length > 1) }));
  // Opening focuses the first group's choice. Without it Chrome focuses the
  // body instead while it scrolls, on a narrow screen.
  const first = shown[0]!.groups[0]!.name;
  return html`<dialog id="options-dialog" class="options columns" aria-labelledby="options-title" ref=${dialog} ...${backdrop()}>
<form id="options" method="dialog">
  <header>
    <h2 class="section" id="options-title">options</h2>
    <p class="label">kept in this browser. changes apply at once.</p>
  </header>
  <div class="options-body">
    ${shown.map((column) => html`<section key=${column.heading} aria-labelledby=${`options-${column.heading}`}>
      <h3 class="section" id=${`options-${column.heading}`}>${column.heading}</h3>
      ${column.groups.map((group) => html`<fieldset key=${group.name}>
        <legend>${group.legend}</legend>
        ${group.choices.map(([value, label]) => html`<label class="choice"><input type="radio" name=${group.name} value=${value} checked=${prefs.read(group.name) === value} autofocus=${group.name === first && prefs.read(group.name) === value} onChange=${() => choose(group.name, value)} /> ${label}</label>`)}
        ${group.name === "notify" && html`<p class="label" id="notify-status" role="status">${STATUS[allowed]}</p>
        <button type="button" id="notify-allow" hidden=${allowed !== "default"} onClick=${ask}>allow notifications</button>`}
      </fieldset>`)}
    </section>`)}
  </div>
  <div class="actions"><button type="button" id="options-reset" onClick=${reset}>reset</button><button type="submit">close</button></div>
</form>
</dialog>`;
}

// The keys, which `?` shows.
export function Keys({ dialog }: { dialog: Ref<HTMLDialogElement> }) {
  return html`<dialog id="keys-dialog" class="options" aria-labelledby="keys-title" ref=${dialog} ...${backdrop()}>
<form method="dialog">
  <h2 class="section" id="keys-title">keys</h2>
  <dl class="keys">
    <dt><kbd>⌘</kbd> <kbd>Enter</kbd> or <kbd>Ctrl</kbd> <kbd>Enter</kbd></dt><dd>send the answer, even from a text field</dd>
    <dt><kbd>1</kbd>–<kbd>9</kbd></dt><dd>choose that option (decision, sign-off, stuck)</dd>
    <dt><kbd>1</kbd> <kbd>2</kbd> <kbd>3</kbd></dt><dd>approve, hold or don't run the current row of an approval, then go to the next row with none</dd>
    <dt><kbd>n</kbd></dt><dd>go to the note to the agent</dd>
    <dt><kbd>j</kbd> <kbd>k</kbd></dt><dd>next or previous case in the list</dd>
    <dt><kbd>?</kbd></dt><dd>this list</dd>
  </dl>
  <p class="label">only the send keys work while typing in a field.</p>
  <div class="actions"><button type="submit">close</button></div>
</form>
</dialog>`;
}

// The number of the last notification shown, kept per browser with serve's
// boot id, so that tabs share it and a restarted serve starts it again.
const CURSOR = "cases.notify-cursor";

// How often a tab with notifications on asks for them, shown or not.
const POLL = 5000;

// This tab's copy of the cursor, for when localStorage cannot be used.
let cursor: Cursor | null = null;

function readCursor(): Cursor | null {
  try {
    const c = JSON.parse(localStorage.getItem(CURSOR)!);
    if (c && typeof c.boot === "string" && typeof c.id === "number") return c;
  } catch (e) {
    // localStorage cannot be read: this tab's copy stands in.
  }
  return cursor;
}

function writeCursor(c: Cursor | null): void {
  cursor = c;
  try {
    if (c) localStorage.setItem(CURSOR, JSON.stringify(c));
    else localStorage.removeItem(CURSOR);
  } catch (e) {
    // localStorage cannot be written: this tab's copy stands in.
  }
}

// Shows `item`, unless notifications are on for blocking cases only and it
// is not one. Its tag is the same in every tab, so the desktop shows it
// once; a click opens the case with `open`. One the browser will not make
// (Chrome on Android makes them only from a service worker) is passed over,
// and the cursor moves past it all the same: each item is tried once,
// shown or not.
function show(item: Item, open: (path: string) => void): void {
  if (prefs.read("notify") === "blocking" && !(item.case && item.case.urgency === "blocking")) return;
  let notification: Notification;
  try {
    notification = new Notification(item.title, { body: item.body, tag: item.tag });
  } catch {
    return;
  }
  notification.onclick = () => {
    window.focus();
    open(item.url);
    notification.close();
  };
}

// Asks serve every few seconds for the cases that landed on the human, while
// notifications are on and allowed, and shows each new one (IN-08). Nothing
// is asked while they are off, and the kept number is dropped then, so
// turning them on later does not replay what came in meanwhile.
export function notify(open: (path: string) => void): void {
  const check = async () => {
    let again = false;
    try {
      if (prefs.read("notify") === "off" || permission() !== "granted") {
        writeCursor(null);
        return;
      }
      const have = readCursor();
      const response = await api.notifications(have ? have.id : null);
      if (!response.ok) return;
      const page = response.body;
      // Read again: another tab may have shown these while this one waited.
      const c = readCursor();
      // A tab with no number kept starts at the latest, and shows nothing.
      if (!c) {
        writeCursor({ boot: page.boot, id: page.latest });
        return;
      }
      // serve restarted: its feed holds only what came after it started, so
      // the new boot starts from 0, and asks again now.
      if (c.boot !== page.boot) {
        writeCursor({ boot: page.boot, id: 0 });
        again = true;
        return;
      }
      // This tab asked after a number another tab has since moved past or
      // started again, so it could miss items: ask again.
      if (have && (have.boot !== c.boot || have.id > c.id)) {
        again = true;
        return;
      }
      if (c.id >= page.latest) return;
      for (const item of page.items) if (item.id > c.id) show(item, open);
      writeCursor({ boot: page.boot, id: page.latest });
    } catch (e) {
      // serve cannot be reached: ask again at the next check.
    } finally {
      setTimeout(check, again ? 0 : POLL);
    }
  };
  check();
}
