// The app's pages and their addresses (IN-01, IN-02, IN-07, IN-13): the list
// under a filter, `/` or `/?show=done`, beside its first case; a case beside
// the list, `/cases/{id}` or `/cases/{id}?show=done`; and the composer of a
// message beside the list, `/message`, with `?to=` a worker and `?re=` the
// case it is about. Plain functions of an address; web/test/routes.test.js
// runs them in node.
import { idOf } from "./ids.js";
import type { Case } from "./types.js";

// The list's filters: the inbox, the default; the cases with an agent; the
// cases closed or withdrawn; and every case. Briefings are in none of these
// until they are closed or withdrawn: they have a tab of their own,
// briefings (IN-14).
export type Filter = "inbox" | "inflight" | "done" | "all" | "briefings";

// The filters, in the order the list's filter row draws them.
export const FILTERS: { show: Filter; label: string }[] = [
  { show: "inbox", label: "inbox" },
  { show: "inflight", label: "in flight" },
  { show: "done", label: "done" },
  { show: "all", label: "all" },
  { show: "briefings", label: "briefings" },
];

// A page of the app. The list's filter is `show`; a case's page without
// one lists the case under its own filter: see under. The composer's `to`
// and `re` say who a message goes to and what it is about.
export interface Route {
  page: "list" | "case" | "compose";
  id?: string;
  show: Filter | null;
  to?: string;
  re?: string;
}

// The filter `show` names, or null when it names none.
export function filter(show: string | null): Filter | null {
  return FILTERS.some((f) => f.show === show) ? (show as Filter) : null;
}

// The filter case `c` is listed under by its state. A message the human
// sent is in flight until an agent closes it. A briefing is under its own
// tab until it is closed or withdrawn (IN-14).
export function under(c: Case): Filter {
  if (c.kind === "briefing" && c.state !== "closed" && c.state !== "withdrawn") return "briefings";
  if (c.state === "open" || c.state === "parked") return "inbox";
  if (c.state === "answered" || c.state === "pickedup" || c.state === "sent") return "inflight";
  return "done";
}

// The query that asks for the filter `show`: none for the inbox.
export function query(show: Filter): string {
  return show === "inbox" ? "" : `?show=${show}`;
}

// What `url` shows, or null for a path that is none of the app's pages,
// which serve answers with a 404. /done, where the done list was, is the
// list under the done filter, or the in-flight one it had.
export function where(url: { pathname: string; search: string }): Route | null {
  const params = new URLSearchParams(url.search);
  const show = filter(params.get("show"));
  if (url.pathname === "/") return { page: "list", show: show || "inbox" };
  if (url.pathname === "/message") {
    return { page: "compose", show, to: params.get("to") || "", re: idOf(params.get("re") || "") };
  }
  if (url.pathname === "/done") return { page: "list", show: show === "inflight" ? "inflight" : "done" };
  const found = /^\/cases\/([^/]+)$/.exec(url.pathname);
  return found ? { page: "case", id: idOf(decoded(found[1]!)), show } : null;
}

// The address to put in place of `url`'s, or null when it is the page's
// own: /done's is that of the filter it is.
export function moved(url: { pathname: string; search: string; hash: string }): string | null {
  const r = where(url);
  return url.pathname === "/done" && r ? `/${query(r.show!)}${url.hash}` : null;
}

// A path segment decoded, or as it is when it is not whole escapes, such as
// `%FF`: no case has that id, and the API says so.
function decoded(segment: string): string {
  try {
    return decodeURIComponent(segment);
  } catch (e) {
    return segment;
  }
}

// Whether routes `a` and `b` show the same page.
export function same(a: Route, b: Route): boolean {
  return a.page === b.page && a.id === b.id && a.show === b.show && a.to === b.to && a.re === b.re;
}

// The composer's address, for a message to the worker `to` about the case
// `re`, either of which may be empty.
export function compose(to = "", re = ""): string {
  const params = new URLSearchParams();
  if (to) params.set("to", to);
  if (re) params.set("re", re);
  const q = params.toString();
  return q ? `/message?${q}` : "/message";
}
