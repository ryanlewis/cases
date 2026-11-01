import { test } from "node:test";
import assert from "node:assert/strict";
import { followUp } from "../static/followup.ts";
import { decision, steps } from "./case.js";

const { open, answer, pickup, note } = steps;

test("a case reopened by the agent's note after an answer is a follow-up", () => {
  const fu = followUp(decision("open", open, answer, pickup, note));
  assert.deepEqual(fu, { answer, notes: [note] });
});

test("a note on an answered case not yet picked up makes a follow-up too", () => {
  assert.deepEqual(followUp(decision("open", open, answer, note)), { answer, notes: [note] });
});

test("the notes are those since the last answer, oldest first", () => {
  const again = { ...answer, seq: 5, data: { other: true, note: "is this stale?" } };
  const second = { ...note, seq: 7, data: { body: "Follow-up: sure?" } };
  const third = { ...note, seq: 8, data: { body: "and the budget?" } };
  const fu = followUp(decision("open", open, answer, pickup, note, again, { ...pickup, seq: 6 }, second, third));
  assert.deepEqual(fu, { answer: again, notes: [second, third] });
});

test("a case never answered is not a follow-up, notes or not", () => {
  assert.equal(followUp(decision("open", open)), null);
  assert.equal(followUp(decision("open", open, { ...note, seq: 2 })), null);
});

test("a case that is not open is not a follow-up", () => {
  for (const state of ["parked", "answered", "pickedup", "closed", "withdrawn"]) {
    assert.equal(followUp(decision(state, open, answer, pickup, note)), null, state);
  }
});

test("an amend after the note asks afresh, so the case is not a follow-up", () => {
  const amend = { seq: 5, author: "agent", event: "amend", file: "", data: { options: ["a book"] } };
  assert.equal(followUp({ ...decision("open", open, answer, pickup, note, amend), amend_seq: 5 }), null);
});

test("an amend after the note that only adds a label leaves it a follow-up", () => {
  const amend = { seq: 5, author: "agent", event: "amend", file: "", data: { labels: ["gifts"] } };
  assert.deepEqual(followUp(decision("open", open, answer, pickup, note, amend)), { answer, notes: [note] });
});

test("an amend that reopens the case asks afresh, so the case is not a follow-up", () => {
  const amend = { seq: 4, author: "agent", event: "amend", file: "", data: { title: "A watch or a print?", reopen: true } };
  assert.equal(followUp(decision("open", open, answer, pickup, amend)), null);
});

test("a human's park and resume after the note leave it a follow-up", () => {
  const park = { seq: 5, author: "human", event: "park", file: "", data: {} };
  const resume = { seq: 6, author: "human", event: "resume", file: "", data: {} };
  assert.deepEqual(followUp(decision("open", open, answer, pickup, note, park, resume)), { answer, notes: [note] });
});

test("a case with no events is not a follow-up", () => {
  assert.equal(followUp({ ...decision("open"), events: undefined }), null);
});
