import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { parse } from "yaml";

// Every workflow file, parsed exactly as GitHub reads it, by file name.
const directory = new URL("../workflows/", import.meta.url);
export function definitions() {
  return Object.fromEntries(
    readdirSync(directory)
      .filter((name) => name.endsWith(".yml"))
      .map((name) => [name, parse(readFileSync(new URL(name, directory), "utf8"))]),
  );
}

// `refuses(change, expected)` for a spec whose `problems(all)` lists what is wrong with a set
// of workflows: `change` applied to the real ones must make one problem match `expected`.
export function refusals(problems) {
  return (change, expected) => {
    const all = definitions();
    change(all);
    const found = problems(all);
    assert.ok(
      found.some((problem) => expected.test(problem)),
      `expected a problem matching ${expected}, got ${JSON.stringify(found)}`,
    );
  };
}
