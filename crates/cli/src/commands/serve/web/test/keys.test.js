import { test } from "node:test";
import assert from "node:assert/strict";
import { rowAfter, rowFor } from "../static/keys.ts";

test("the number keys set the approval row with the focus, else the first with no verdict", () => {
  assert.equal(rowFor([false, false, false], -1), 0);
  assert.equal(rowFor([true, false, false], -1), 1);
  assert.equal(rowFor([true, true, false], 0), 0);
  assert.equal(rowFor([false, true, false], 1), 1);
  assert.equal(rowFor([true, true, true], -1), -1);
});

test("after a verdict the focus goes to the next row with none, or stays", () => {
  assert.equal(rowAfter([true, false, false], 0), 1);
  assert.equal(rowAfter([true, true, false], 0), 2);
  assert.equal(rowAfter([true, true, true], 1), 1);
  // An earlier row with no verdict is not gone back to.
  assert.equal(rowAfter([false, true, true], 2), 2);
  assert.equal(rowAfter([false, true, true], 1), 1);
});
