// A case's id as people see it on the page: in its address, with its
// title's slug after it, `/cases/UkLWZ-pin-bun-or-float` (ST-10, case
// 2026-09-25T20-55-32Z-title-slug-after-ids-show-it-in-v1-lists-and).
// Lookups ignore the slug, so an address whose slug is out of date after
// an amend still finds the case.

// The lowercase ASCII words of `title`, joined by `-`: `case` when it has
// none, and cut at a word to 60 characters. The same as the CLI's slug
// (output.rs), which a test holds them to.
export function slug(title: string | undefined): string {
  let s = "";
  for (const c of (title || "").toLowerCase()) {
    if (/^[a-z0-9]$/.test(c)) s += c;
    else if (s && !s.endsWith("-")) s += "-";
  }
  s = s.replace(/-+$/, "");
  if (s.length > 60) {
    s = s.slice(0, 60);
    const dash = s.lastIndexOf("-");
    if (dash > 0) s = s.slice(0, dash);
  }
  return s || "case";
}

// The page of the case `c`, which has an id and a title.
export function page(c: { id: string; title: string }): string {
  return `${pageOf(c.id)}-${slug(c.title)}`;
}

// The page of the case with id `id`, when its title is not known: the
// address without a slug, which finds the case as well.
export function pageOf(id: string): string {
  return `/cases/${encodeURIComponent(id)}`;
}

// A case named in text as `case:ID`, as the skill tells agents to write
// one: an id in Sqids's alphabet, at least 5 characters (cases-id), with no
// letter, digit or `_` of any script on either side, so `showcase:abcde`,
// `case:abcde_x` and `case:abcdeé` name no case. Its first group is the id.
export const CASE_REF = /(?<![\p{L}\p{N}_])case:([A-Za-z0-9]{5,})(?![\p{L}\p{N}_])/gu;

// Whether text ending as `text` does is joined to a `case:` straight after
// it, by the rule CASE_REF looks back with.
export const JOINED = /[\p{L}\p{N}_]$/u;

// The id in a page's path segment: the text before the slug, if any. An id
// never holds a `-`.
export function idOf(segment: string): string {
  const at = segment.indexOf("-");
  return at < 0 ? segment : segment.slice(0, at);
}
