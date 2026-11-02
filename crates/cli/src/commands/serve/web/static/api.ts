// The inbox's API (spec/api.md), as the page uses it.
import type { Case, Category, Index, List, Listener, Listeners, NewMessage, Notifications, Refusal, ServedCase, Write } from "./types.js";

export type { NewMessage };

// What a write records: one to a case, or a message the human sends.
export type What = "answer" | "park" | "resume" | "withdraw" | "amend" | "message";

// A response: whether it succeeded, its status, and its body as JSON: the
// route's shape when it succeeded, else a refusal, or null when the body is
// not JSON.
export type Reply<T> =
  | { ok: true; status: number; body: T & { error?: never } }
  | { ok: false; status: number; body: Refusal | null };

// The CSRF token every write carries, from GET /v1 (IN-11).
let token = "";

// GET /v1: the build serve is, and the token for writes.
export async function index(): Promise<Index> {
  const response = await fetch("/v1", { cache: "no-store" });
  if (!response.ok) throw new Error(`GET /v1: ${response.status}`);
  const body: Index = await response.json();
  token = body.csrf;
  return body;
}

// A response: whether it succeeded, its status, and its body as JSON.
async function read<T>(response: Response): Promise<Reply<T>> {
  let body: unknown = null;
  try {
    body = await response.json();
  } catch (e) {
    body = null;
  }
  return { ok: response.ok, status: response.status, body } as Reply<T>;
}

// GET `path` under /v1. A failure to reach serve throws.
async function get<T>(path: string): Promise<Reply<T>> {
  return read<T>(await fetch(`/v1/${path}`, { cache: "no-store" }));
}

// A read serve answered with a refusal: serve was reached, and says why.
export class Refused extends Error {
  declare status: number;
  constructor(response: Reply<unknown>) {
    super(message(response));
    this.status = response.status;
  }
}

// The cases in `states`, in inbox order: most urgent first, then in the
// order they were opened. Throws unless serve answers with the list:
// Refused when serve answers with a refusal.
export async function list(states: readonly string[]): Promise<Case[]> {
  const response = await get<List>(`cases?state=${encodeURIComponent(states.join(","))}`);
  if (!response.ok) throw new Refused(response);
  return response.body.list.map(received);
}

// The case `id`, as show --json gives it; `ok` is false when there is none.
export async function one(id: string): Promise<Reply<Case>> {
  return receivedReply(await get<ServedCase>(`cases/${encodeURIComponent(id)}`));
}

// Case `c` as serve sent it, with empty data on each event whose record
// has none, as the page reads an event: the page never meets an event with
// no data, wherever it reads one.
export function received(c: ServedCase): Case {
  // A case serve sent without events (as in a write's reply) has none to
  // give data to, and is left as it is.
  if (!c.events) return c as Case;
  return { ...c, events: c.events.map((e) => ({ ...e, data: e.data || {} })) };
}

// A reply whose body is a case as serve sent it, with the case as received.
function receivedReply(response: Reply<ServedCase>): Reply<Case> {
  return response.ok ? { ...response, body: received(response.body) } : response;
}

// The workers listening for messages now (ST-16); null when that is not
// known, as the store cannot be read or serve is out of reach. The page does
// not fail for it, and does not take it for no one listening.
export async function listeners(): Promise<Listener[] | null> {
  try {
    const response = await get<Listeners>("listeners");
    return response.ok ? response.body.list : null;
  } catch (e) {
    return null;
  }
}

// The notifications after `after` (IN-08).
export function notifications(after: number | null): Promise<Reply<Notifications>> {
  return get<Notifications>(after === null ? "notifications" : `notifications?after=${after}`);
}

// Sends `write` (answer, park, resume, withdraw or amend) of `body` to the
// case `id`.
export function write(id: string, what: Exclude<What, "message">, body: Write): Promise<Reply<Case>> {
  return post(`cases/${encodeURIComponent(id)}/${what}`, body);
}

// POSTs `body` to `path` under /v1, with the CSRF token. A token refused as
// it is from before serve restarted is read again, and the write sent once
// more: nothing was written.
async function post(path: string, body: unknown): Promise<Reply<Case>> {
  return receivedReply(await postServed(path, body));
}

// POSTs as post does, and gives the case in the reply as serve sent it.
async function postServed(path: string, body: unknown): Promise<Reply<ServedCase>> {
  if (!token) await index();
  const send = () =>
    fetch(`/v1/${path}`, {
      method: "POST",
      headers: { "Content-Type": "application/json", "X-Cases-CSRF": token },
      body: JSON.stringify(body),
      cache: "no-store",
    }).then(read<ServedCase>);
  let response = await send();
  if (response.status === 403 && category(response) === "csrf") {
    await index();
    response = await send();
  }
  return response;
}

// Sends the human's message, `body`, and says what came of it, as send
// does: the message as it is once sent, or why it was refused. A message has
// no revision to refuse it twice, and serve finishes a write whose answer
// never came back, so when serve cannot be reached it may have been sent.
export async function sendMessage(body: NewMessage): Promise<Sent> {
  try {
    const response = await post("cases", body);
    if (response.ok) return { after: response.body };
    return { refusal: message(response) };
  } catch (e) {
    return { refusal: "serve could not be reached, so the message may or may not have been sent. look for it in flight before sending it again." };
  }
}

// What a send came to: the case after the write, or why it was refused,
// for people, with the case as it is now when serve gave it.
export type Sent = { after: Case; refusal?: never; current?: never } | { after?: never; refusal: string; current?: Case };

// Sends `what` of `body` to the case `id`, as write does, and says what came
// of it. A stale refusal says so plainly; serve out of reach is a refusal too,
// one that says the outcome is not known and gives no case. A write is never
// sent again for it.
export async function send(id: string, what: Exclude<What, "message">, body: Write): Promise<Sent> {
  try {
    const response = await write(id, what, body);
    if (response.ok) return { after: response.body };
    const refusal =
      category(response) === "stale"
        ? "this case changed after it was drawn, so this was not recorded. check the thread before sending again."
        : message(response);
    const current = response.body && response.body.error && response.body.error.case;
    return current ? { refusal, current: received(current) } : { refusal };
  } catch (e) {
    // The request may have reached serve, which finishes a write whose
    // answer never came back, so what came of it is not known. No case is
    // given: a form that took the revision of a case that already holds this
    // write would send it again and have it accepted. The form keeps its
    // revision, so a second send is refused as stale if the first was
    // recorded, and the stream redraws the case when serve is back.
    return { refusal: "serve could not be reached, so this may or may not have been recorded. check the thread before sending again." };
  }
}

// What a page of build `build` does when serve says it is build `served`
// (IN-12): nothing when they are the same, else reload, or, when something
// typed would be lost, say that a new version is running.
export function upgrade(served: string, build: string, typed: boolean): "same" | "reload" | "say" {
  if (served === build) return "same";
  return typed ? "say" : "reload";
}

// The category of a refusal, such as "stale".
export function category(response: Reply<unknown>): Category | "" {
  return response.body && response.body.error ? response.body.error.category : "";
}

// What a refusal says, for people.
export function message(response: Reply<unknown>): string {
  if (response.body && response.body.error) return response.body.error.message;
  return `serve answered ${response.status}`;
}
