// The shapes of serve's API (spec/api.md), as the page reads and sends them.
// output.rs (`CaseJson`) and the core's records (crates/core/src/record.rs)
// are the source of truth: a field they leave out when it is empty is
// optional here. Types only: build.rs strips nothing from this file, and it
// is not served.

// A time as the store writes it: RFC 3339, in UTC.
export type Time = string;

export type State = "open" | "parked" | "answered" | "pickedup" | "closed" | "withdrawn" | "sent";
export type Kind = "decision" | "approval" | "signoff" | "stuck" | "question" | "fyi" | "briefing" | "message";
export type Urgency = "blocking" | "today" | "whenever";
export type Author = "agent" | "human";
export type EventName =
  | "open"
  | "amend"
  | "answer"
  | "pickup"
  | "note"
  | "close"
  | "withdraw"
  | "park"
  | "resume";

// An actor, named from the store's actors table: `{ id }` alone when its
// row is gone (IN-02).
export interface Actor {
  id: string;
  kind?: string;
  name?: string;
}

// An actor as a record stores it: its id only.
export interface ActorRef {
  id?: string;
}

// A row of an approval. The core refuses a row without an id or a label.
export interface ApprovalRow {
  id: string;
  label: string;
  script?: string;
  link?: string;
  note?: string;
  // "human" on a step the human carries out, where approve means done.
  // Left out on a step the agent runs.
  by?: "human";
}

// An approval row's verdict in an answer.
export interface RowVerdict {
  id: string;
  verdict: "approve" | "hold" | "reject";
  note?: string;
}

// The answer record (HU-01, HU-02).
export interface Answer {
  choice?: number;
  other?: boolean;
  rows?: RowVerdict[];
  signoff?: "accept" | "changes";
  // A briefing's verdict, and its left-out items to file, by number from 1.
  briefing?: "accept" | "send_back";
  file?: number[];
  text?: string;
  drop?: boolean;
  ack?: boolean;
  note?: string;
  amend_seq?: number;
  actor?: ActorRef;
  answered_at?: Time;
}

export interface Pickup {
  by?: string;
  actor?: ActorRef;
  picked_up_at?: Time;
}

export interface Park {
  note?: string;
  actor?: ActorRef;
  parked_at?: Time;
}

export interface Close {
  outcome?: string;
  links?: string[];
  actor?: ActorRef;
  closed_at?: Time;
}

// An event's record as stored, unknown fields included. Which fields it
// has depends on the event; `rows` are an open's or amend's approval rows,
// or an answer's verdicts.
export interface EventData {
  title?: string;
  body?: string;
  context?: string;
  options?: string[];
  recommend?: number;
  rows?: (ApprovalRow & Partial<RowVerdict>)[] | (RowVerdict & Partial<ApprovalRow>)[];
  // A briefing's open or amend: whether its work has landed, and what it
  // left out.
  landed?: boolean;
  left_out?: string[];
  links?: string[];
  // Links an amend took off the case.
  drop_links?: string[];
  labels?: string[];
  // A note or an amend that says it reopens an answered or picked-up case.
  reopen?: boolean;
  worker?: string;
  choice?: number;
  other?: boolean;
  signoff?: string;
  // A briefing's answer: its verdict and the left-out items to file.
  briefing?: string;
  file?: number[];
  text?: string;
  drop?: boolean;
  ack?: boolean;
  note?: string;
  by?: string;
  outcome?: string;
  reason?: string;
  [field: string]: unknown;
}

// An event of a case.
export interface Event {
  seq: number;
  author: Author;
  event: EventName;
  // The case's state before the event; left out on the open.
  from?: State;
  file: string;
  at?: Time;
  actor?: Actor;
  data: EventData;
  // The record's body, left out of `data`, is the case's `body`: an open or
  // amend that set the body the case has now.
  body_in_case?: boolean;
}

// An event as serve sends it: one whose record has no data has null for
// it. api.ts gives each such event empty data before the page reads it, so
// the page reads `Event`, whose data is always there.
export interface ServedEvent extends Omit<Event, "data"> {
  data: EventData | null;
}

// A case, as `show --json` gives it (AG-13), and each item of a list.
export interface Case {
  id: string;
  state: State;
  kind: Kind;
  urgency: Urgency;
  title: string;
  body?: string;
  options?: string[];
  // The option the agent recommends, numbered from 1; left out when none.
  recommend?: number;
  rows?: ApprovalRow[];
  // A briefing's only: whether its work has landed, and what it left out,
  // numbered from 1 by an answer's `file`.
  landed?: boolean;
  left_out?: string[];
  links?: string[];
  labels?: string[];
  // The worker that opened the case; on a message, the worker it is sent
  // to.
  worker?: string;
  brief?: string;
  context?: string;
  for?: string;
  // The id of the case this one is about, such as the message an agent's
  // question follows.
  re?: string;
  actor?: Actor;
  opened_at?: Time;
  updated_at?: Time;
  answer?: Answer;
  pickup?: Pickup;
  park?: Park;
  close?: Close;
  events: Event[];
  problems?: string[];
  revision: number;
  amend_seq: number;
  // The highest seq among the case's rows, skipped ones included.
  last_seq: number;
  url?: string;
}

// A case as serve sends it, its events as serve sent them (`ServedEvent`).
export interface ServedCase extends Omit<Case, "events"> {
  events: ServedEvent[];
}

// `GET /v1/cases`.
export interface List {
  list: ServedCase[];
}

// A worker listening for messages now (ST-16): its labels, when it began
// listening, when a wait of its last refreshed its record, what it says it
// is doing, since when, and its waits listening now.
export interface Listener {
  worker: string;
  labels: string[];
  since: Time;
  heard: Time;
  doing?: string;
  doing_at?: Time;
  waits: ListenerWait[];
}

// One wait of a listener: the labels and kinds it waits on, none of either
// for any.
export interface ListenerWait {
  labels: string[];
  kinds: string[];
}

// `GET /v1/listeners`.
export interface Listeners {
  list: Listener[];
}

// `POST /v1/cases`: the message the human sends, as its open record.
export interface NewMessage {
  kind: "message";
  urgency: Urgency;
  title: string;
  body?: string;
  worker?: string;
  labels?: string[];
  links?: string[];
  context?: string;
  re?: string;
}

// `GET /v1`.
export interface Index {
  version: string;
  csrf: string;
}

// A refusal's category: the core's, `csrf`, `request`, or `store` when
// the store failed.
export type Category =
  | "stale"
  | "invalid"
  | "transition"
  | "unknown"
  | "author"
  | "malformed"
  | "csrf"
  | "request"
  | "store";

// The body of a refusal. `case` is there on 409 and 422 (IN-05).
export interface Refusal {
  error: {
    category: Category;
    message: string;
    case?: ServedCase;
  };
}

// A write's body: the revision the form was drawn at, and the event's own
// fields: an answer, a park's note, a withdraw's reason, or what the human's
// amend of a message changes.
export interface Write {
  revision: number;
  answer?: Answer;
  note?: string;
  reason?: string;
  body?: string;
}

// One notification (IN-08).
export interface Notification {
  id: number;
  sink: string;
  tag: string;
  title: string;
  body: string;
  url: string;
  queued_at: Time;
  event: {
    name: string;
    seq: number;
    author: Author;
    event: EventName;
    file: string;
    at?: Time;
  };
  case: ServedCase;
}

// `GET /v1/notifications`.
export interface Notifications {
  boot: string;
  latest: number;
  items: Notification[];
}

// A display option of theme.ts: its first value is its default.
export interface Option {
  name: string;
  values: string[];
}

// The display preferences theme.ts keeps, as window.casesPrefs (IN-10).
export interface Prefs {
  options: Option[];
  read(name: string): string;
  write(name: string, value: string): void;
  apply(): void;
}

declare global {
  interface Window {
    casesPrefs: Prefs;
  }
}
