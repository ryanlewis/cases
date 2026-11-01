// The one rule for every link the page draws (IN-02): in markdown, and a
// case's and a row's links. An absolute address is a link unless its scheme
// runs code or reaches local or opaque content; anything else, such as a
// relative path or a file's path, is text. So http, https, mailto and an
// app's own scheme, such as obsidian:, are links (case
// 2026-09-25T20-55-32Z-markdown-links-only-http-s-mailto-clickable).
//
// The address is read as the browser reads it: leading and trailing spaces
// and control characters are dropped, tabs and line breaks are dropped
// anywhere, and the scheme is matched in any case, so `JaVa\tscript:` is
// javascript. What is checked is what would be followed.

// The schemes never linked: they run code, or reach local or opaque content.
const BLOCKED = ["javascript", "vbscript", "data", "file", "blob", "filesystem", "about"];

// The address `href` goes to, if it is one the page may link to; null when
// it is relative, does not read as an address, or its scheme is blocked.
export function link(href: string): URL | null {
  let url;
  try {
    url = new URL(href);
  } catch (e) {
    return null;
  }
  return BLOCKED.includes(url.protocol.slice(0, -1)) ? null : url;
}
