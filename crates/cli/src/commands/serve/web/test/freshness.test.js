// The web's port of the core's freshness rule (fresh.ts `since`), run over
// the core's vectors (spec/vectors/freshness.json), which a Rust test renders
// as the API gives each case into freshness.json beside this file.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { drawnFrom, redrawn } from "../static/case.ts";
import { followed, mark, since } from "../static/fresh.ts";
import { decision } from "./case.js";

const { vectors } = JSON.parse(readFileSync(new URL("./freshness.json", import.meta.url), "utf8"));

// The vector described as `description`.
function vector(description) {
  const v = vectors.find((v) => v.description === description);
  assert.ok(v, `no vector ${JSON.stringify(description)}`);
  return v;
}

test("the fixture has vectors of each outcome", () => {
  const outcomes = new Set(vectors.map((v) => v.since));
  assert.deepEqual([...outcomes].sort(), ["changed", "moved", "unchanged"]);
});

for (const v of vectors) {
  test(`freshness: ${v.description}`, () => {
    assert.equal(since(v.case, v.drawn), v.since);
    // A form drawn from the case keeps the revision and seq the core does.
    if (v.drawn_case) {
      const { revision, seq } = drawnFrom(v.drawn_case);
      assert.deepEqual({ revision, seq }, v.drawn);
    }
  });
}

test("a form follows the human's own edit of a sent message, so its withdraw is not refused", () => {
  const v = vector("the human's edit of a sent message's body: unchanged");
  const was = drawnFrom(v.drawn_case);
  assert.equal(v.case.state, "sent");
  assert.deepEqual(redrawn(was, v.case), drawnFrom(v.case));
});

test("a form drawn before a park and a resume between two reads keeps its revision, so its send is refused as stale", () => {
  const v = vector("parked and resumed: changed");
  const was = drawnFrom(v.drawn_case);
  assert.equal(v.case.state, v.drawn_case.state);
  assert.deepEqual(redrawn(was, v.case), was);
});

test("a form in the state it was drawn in takes the new revision only when the case is unchanged since", () => {
  for (const v of vectors) {
    if (!v.drawn_case || v.case.state !== v.drawn_case.state) continue;
    const was = drawnFrom(v.drawn_case);
    const want = v.since === "unchanged" ? drawnFrom(v.case) : was;
    assert.deepEqual(redrawn(was, v.case), want, v.description);
  }
});

// A row's withdraw, drawn on an answered or picked-up case, follows it as the
// case view's forms do, in the state it was drawn in: it takes the new
// revision only when the case is unchanged since, as after the agent's note,
// and keeps its old one otherwise, so that its send is refused as stale.
test("a row's withdraw follows the agent's note on a picked-up case, and keeps its revision after a reopen and an answer", () => {
  const open = { seq: 1, author: "agent", event: "open", file: "", at: "2026-09-25T13:10:00Z", data: {} };
  const answer = { seq: 2, author: "human", event: "answer", from: "open", file: "", at: "2026-09-25T13:20:00Z", data: { choice: 1 } };
  const pickup = { seq: 3, author: "agent", event: "pickup", from: "answered", file: "", at: "2026-09-25T13:21:00Z", data: { by: "gift-agent" } };
  const note = { seq: 4, author: "agent", event: "note", from: "pickedup", file: "", at: "2026-09-25T13:30:00Z", data: { body: "Rebasing first." } };
  const drawn = decision("pickedup", open, answer, pickup);
  const noted = decision("pickedup", open, answer, pickup, note);
  assert.equal(since(noted, mark(drawn)), "unchanged");
  assert.deepEqual(followed(mark(drawn), noted), mark(noted));
  const again = vector("answered elsewhere and reopened between two reads: changed");
  assert.deepEqual(followed(mark(again.drawn_case), again.case), mark(again.drawn_case));
  for (const v of vectors) {
    if (!v.drawn_case || v.case.state !== v.drawn_case.state) continue;
    const want = v.since === "unchanged" ? mark(v.case) : v.drawn;
    assert.deepEqual(followed(v.drawn, v.case), want, v.description);
  }
});
