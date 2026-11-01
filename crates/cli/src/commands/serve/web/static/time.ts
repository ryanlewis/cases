// Times for people: in the browser's own zone, with an age (IN-02). The
// store keeps every time in UTC.
import type { Time } from "./types.js";

const MINUTE = 60 * 1000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

// How long ago `at` was, coarsely: "now", then minutes, hours up to two
// days, then days. Empty when there is no time.
export function age(at: Time | number | undefined, now: number = Date.now()): string {
  if (!at) return "";
  const d = now - new Date(at).getTime();
  if (d < MINUTE) return "now";
  if (d < HOUR) return `${Math.floor(d / MINUTE)}m`;
  if (d < 2 * DAY) return `${Math.floor(d / HOUR)}h`;
  return `${Math.floor(d / DAY)}d`;
}

// The age as a phrase: "5m ago", or "just now".
export function ago(at: Time | undefined, now?: number): string {
  const a = age(at, now);
  return a === "now" ? "just now" : `${a} ago`;
}

// `at` in the browser's zone, to the minute, with the zone's short name,
// such as "2026-09-25 14:03 BST".
export function stamp(at: Time | undefined): string {
  if (!at) return "";
  const d = new Date(at);
  const pad = (n: number) => String(n).padStart(2, "0");
  const zone = new Intl.DateTimeFormat(undefined, { timeZoneName: "short" })
    .formatToParts(d)
    .find((part) => part.type === "timeZoneName");
  const day = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
  return `${day} ${pad(d.getHours())}:${pad(d.getMinutes())}${zone ? " " + zone.value : ""}`;
}

// Midnight at the start of today, in the browser's zone (IN-07).
export function today(now: number = Date.now()): number {
  const d = new Date(now);
  return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
}

// Midnight at the start of this week, on Monday, in the browser's zone.
export function thisWeek(now: number = Date.now()): number {
  const day = new Date(today(now));
  const sinceMonday = (day.getDay() + 6) % 7;
  return new Date(day.getFullYear(), day.getMonth(), day.getDate() - sinceMonday).getTime();
}

// Whether `at` is at or after `since`, a time in milliseconds.
export function since(at: Time | undefined, from: number): boolean {
  return !!at && new Date(at).getTime() >= from;
}
