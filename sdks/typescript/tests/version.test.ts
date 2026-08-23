import { expect, test } from "bun:test";
import manifest from "../package.json" with { type: "json" };
import { VERSION } from "../src/index.ts";

test("the exported version matches the published package version", () => {
  // A generated surface pinned to a version that disagrees with the published
  // package is a bug; reading package.json from the tree catches a hand-edited
  // constant that was never released.
  expect(VERSION).toBe(manifest.version);
});

test("the version is a plain semantic version", () => {
  expect(VERSION).toMatch(/^\d+\.\d+\.\d+$/);
});
