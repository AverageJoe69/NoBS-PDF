import assert from "node:assert/strict";
import test from "node:test";
import {
  canonicalLicenceKey,
  formatLicenceBody,
  normaliseLicenceBody,
} from "../src/licence-input.ts";

const canonical = "NOBS-AB12-CD34-EF56-7890";

test("pasting a complete key removes the displayed prefix", () => {
  assert.equal(formatLicenceBody(canonical), "AB12-CD34-EF56-7890");
});

test("pasting a prefix-free key works normally", () => {
  assert.equal(formatLicenceBody("AB12-CD34-EF56-7890"), "AB12-CD34-EF56-7890");
});

test("surrounding and copied whitespace is removed", () => {
  assert.equal(formatLicenceBody(" \n nObS-AB12 CD34-EF56-7890\r\n"), "AB12-CD34-EF56-7890");
});

test("manual typing is grouped after the visual prefix", () => {
  assert.equal(formatLicenceBody("ab12cd34ef567890"), "AB12-CD34-EF56-7890");
});

test("the prefix is never duplicated", () => {
  assert.equal(canonicalLicenceKey(canonical), canonical);
});

test("a complete key does not lose final characters", () => {
  assert.equal(normaliseLicenceBody(canonical), "AB12CD34EF567890");
  assert.equal(normaliseLicenceBody(canonical).slice(-4), "7890");
});

test("submission contains exactly one canonical prefix", () => {
  assert.equal(canonicalLicenceKey("nobs-ab12-cd34-ef56-7890"), canonical);
  assert.equal(canonicalLicenceKey("AB12-CD34-EF56-7890"), canonical);
});

test("invalid payload is not silently truncated into a valid key", () => {
  assert.equal(formatLicenceBody("AB12CD34EF567890EXTRA"), "AB12-CD34-EF56-7890-EXTR-A");
  assert.equal(canonicalLicenceKey("AB12CD34EF567890EXTRA"), null);
});
