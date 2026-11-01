// Code blocks (IN-02): a markdown fence, or an approval row's script, in a
// <pre> with a button that wraps its long lines or scrolls them sideways.
// That is the wrap option (theme.ts), one choice for every block, kept in
// this browser; theme.ts sets it when the button is pressed. Beside it, a
// button copies the block's text as written (copyCode).
//
// A diff, a ```diff or ```patch fence or a script that is a unified diff, has
// each line drawn with a class for its kind, which style.css colours. The
// text is as written: each line keeps its own newline and gets no marks, so
// what is copied is what was sent.

// What a line of a diff is: added, removed, a hunk's @@ header or a "\ No
// newline" note, a file's header (diff --git, index, --- and +++), or
// anything else, such as a context line.
export type Kind = "add" | "del" | "hunk" | "head" | "ctx";

// Whether a row's script is a unified diff: it starts with its file headers,
// `--- ` then `+++ `, or with `diff --git`.
export function isDiff(text: string): boolean {
  return /^(--- .*\r?\n\+\+\+ |diff --git )/.test(text);
}

// The lines of `text`, each with its newline, and its kind as a line of a
// diff. Inside a hunk its @@ header's counts say where it ends, so a removed
// line that begins "-- " is not taken for a file's header. Outside one, as in
// a fence of + and - lines with no @@, a line is known by its first
// character, and --- or +++ only begins a file's header as a pair.
export function diffLines(text: string): { kind: Kind; text: string }[] {
  const lines = text.match(/[^\n]*\n|[^\n]+$/g) || [];
  const out: { kind: Kind; text: string }[] = [];
  // The old and new lines left in the hunk.
  let old = 0;
  let neu = 0;
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]!;
    const c = line[0];
    let kind: Kind;
    // A hunk's lines begin with a space, + or -, or are empty. Any other
    // line, such as the next @@ or diff --git when the counts are wrong, as
    // in a diff written by hand, ends the hunk.
    if ((old > 0 || neu > 0) && c !== "\\" && !/^([ +-]|\r?\n)/.test(line)) {
      old = 0;
      neu = 0;
    }
    if (c === "\\") {
      // "\ No newline at end of file", which follows the line it is about,
      // the last of a hunk as often as not.
      kind = "hunk";
    } else if (old > 0 || neu > 0) {
      if (c === "+") {
        kind = "add";
        neu--;
      } else if (c === "-") {
        kind = "del";
        old--;
      } else {
        kind = "ctx";
        old--;
        neu--;
      }
    } else if (line.startsWith("@@")) {
      kind = "hunk";
      const counts = /^@@ -\d+(?:,(\d+))? \+\d+(?:,(\d+))? @@/.exec(line);
      if (counts) {
        old = counts[1] === undefined ? 1 : Number(counts[1]);
        neu = counts[2] === undefined ? 1 : Number(counts[2]);
      }
    } else if (/^(diff |index )/.test(line)) {
      kind = "head";
    } else if (line.startsWith("--- ") && (lines[i + 1] || "").startsWith("+++ ")) {
      kind = "head";
    } else if (line.startsWith("+++ ") && (lines[i - 1] || "").startsWith("--- ") && out[i - 1]!.kind === "head") {
      kind = "head";
    } else if (c === "+") {
      kind = "add";
    } else if (c === "-") {
      kind = "del";
    } else {
      kind = "ctx";
    }
    out.push({ kind, text: line });
  }
  return out;
}

// The copy button's glyph: two sheets, one over the other.
const COPY = '<svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.5"><rect x="5.5" y="5.5" width="8.5" height="8.5" rx="1"/><path d="M10.5 3V2.5a1 1 0 0 0-1-1h-7a1 1 0 0 0-1 1v7a1 1 0 0 0 1 1H3"/></svg>';

// The HTML of a code block of `text`, drawn as a diff when `diff` is true:
// the copy and wrap buttons, then the <pre>. It goes in an element of class
// "code", which the buttons' rules in style.css are under. `wrapped` is
// whether long lines wrap now, for the wrap button's pressed state;
// theme.ts keeps it so.
export function codeBlock(text: string, diff: boolean, wrapped: boolean): string {
  const body = diff
    ? diffLines(text).map(({ kind, text }) => `<span class="d-${kind}">${escape(text)}</span>`).join("")
    : escape(text);
  return `<div class="code-tools"><button type="button" class="copy-code" aria-label="copy" title="copy">${COPY}</button><button type="button" class="wrap-toggle" aria-pressed="${wrapped}" title="wrap long lines">wrap</button></div><pre${diff ? ' class="diff"' : ""}><code>${body}</code></pre>`;
}

// Copies the text of the code block of `button`, a copy button, as it was
// written: a diff's lines with their + and -, as the <code> holds them.
// A block that is not a diff goes without its last newline, which a fence
// never had, so a command pasted into a terminal does not run at once; a
// diff keeps it, as a patch needs it.
//
// Where the page has no clipboard, as over plain http from another host,
// the text is selected and copied the old way, which such a page may
// still do, with the text as it would go to the clipboard rather than the
// selection's; if that fails too, it stays selected, for the browser's own
// copy. The button says which for a moment. What it did: "copied",
// "selected" or "".
export async function copyCode(button: HTMLElement): Promise<string> {
  const code = button.closest(".code")?.querySelector("pre > code");
  if (!code) return "";
  const text = code.textContent || "";
  const diff = !!code.parentElement?.classList.contains("diff");
  const copy = diff ? text : text.replace(/\n$/, "");
  let done = "copied";
  try {
    if (!navigator.clipboard) throw new Error("no clipboard");
    await navigator.clipboard.writeText(copy);
  } catch {
    const range = document.createRange();
    range.selectNodeContents(code);
    const selection = getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
    let copied = false;
    const put = (e: ClipboardEvent) => {
      e.clipboardData?.setData("text/plain", copy);
      e.preventDefault();
    };
    document.addEventListener("copy", put);
    try {
      copied = document.execCommand("copy");
    } catch {
    } finally {
      document.removeEventListener("copy", put);
    }
    done = copied ? "copied" : "selected";
  }
  say(button, done);
  return done;
}

// The glyph of each copy button saying what it did, until it goes back.
const said = new WeakMap<HTMLElement, ReturnType<typeof setTimeout>>();
function say(button: HTMLElement, done: string): void {
  clearTimeout(said.get(button));
  button.textContent = done;
  button.setAttribute("aria-label", done);
  button.setAttribute("title", done);
  said.set(button, setTimeout(() => {
    button.innerHTML = COPY;
    button.setAttribute("aria-label", "copy");
    button.setAttribute("title", "copy");
    said.delete(button);
  }, 1500));
}

// Whether long lines wrap now, as the options have it (theme.ts).
export function wrapped(): boolean {
  return typeof window !== "undefined" && !!window.casesPrefs && window.casesPrefs.read("wrap") === "wrap";
}

function escape(text: string): string {
  return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}
