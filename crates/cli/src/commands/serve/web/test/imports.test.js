// The guard in hooks.js: a static module cannot be imported as a second copy.
import { test } from "node:test";
import assert from "node:assert/strict";

for (const suffix of ["?no-token", "#no-token"]) {
  test(`a static module imported under ${suffix} is refused`, async () => {
    await assert.rejects(import(`../static/api.ts${suffix}`), /a second copy of a static module/);
  });
}
