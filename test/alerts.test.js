import { test } from "node:test";
import assert from "node:assert/strict";
import { gist } from "../src/alerts.js";

test("an alert shows the question at the end of a long message", () => {
  const long = "…levels, with gaps framed as eager. I updated the resume and the cover letter. Should I send both now, or wait for your edits? Either works.";
  assert.equal(gist(long), "Should I send both now, or wait for your edits?");
  assert.equal(gist("Done. All 48 tests pass."), "All 48 tests pass.");
  assert.equal(gist("no punctuation at all"), "no punctuation at all");
  assert.equal(gist("Want me to open a PR?\n\n"), "Want me to open a PR?");
  assert.equal(gist(""), "");
});
