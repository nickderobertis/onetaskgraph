import { expect, test } from "bun:test";
import Ajv2020 from "ajv/dist/2020.js";
import type { LinearConfig } from "../src/index.ts";
import { runtimeSchemas } from "../src/generated/schemas.ts";

// A `linear` source's configuration is a root of the bundle, so a caller writing one has its
// `status_mapping` and its `project` typed, and the runtime schema refuses what the binary would.
test("a linear source's configuration is modelled with its status mapping and its project", () => {
  const configured: LinearConfig = {
    team: "ENG",
    project: "986a467e-775a-4f8f-80dd-aca405063cf4",
    status_mapping: {
      queued: "Queued",
      "in-progress": "In Progress",
      draft: null,
      todo: { task: "Todo", project: "Planned" },
      done: { project: "Completed" },
    },
  };
  const validate = new Ajv2020({ strict: false }).compile(runtimeSchemas.LinearConfig);
  expect(validate(configured)).toBe(true);
  expect(validate({ ...configured, status_mapping: { queued: 7 } })).toBe(false);
  expect(validate({ ...configured, unknown_key: true })).toBe(false);
  // A key that names no status category is refused, as the binary refuses it.
  expect(validate({ ...configured, status_mapping: { shipped: "Done" } })).toBe(false);
  // And so is every per-kind object the grammar refuses: one naming no kind, one naming
  // another key, and one holding a `null` or a blank name.
  for (const refused of [{}, { epic: "Done" }, { task: null }, { project: "" }]) {
    expect(validate({ ...configured, status_mapping: { done: refused } })).toBe(false);
  }
});
