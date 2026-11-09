import "../../test-setup.mjs";
import assert from "node:assert/strict";
import { test } from "node:test";
import { parse } from "yaml";
import { evaluate, read, workflows } from "../../workflow-graph.mjs";

// Only a push to main may save a Rust build cache. A save from any other ref is a cache
// nothing else can restore that still counts against the repository's 10 GiB, and the eviction
// it causes is paid in another pull request's cold build. Every workflow file is read, so a
// workflow added later is held to this without being named here.

const SETUP = "actions-rust-lang/setup-rust-toolchain@";
const RUST_CACHE = "Swatinem/rust-cache@";
const CACHE_SAVE = /^actions\/cache(?:\/save)?@/;
const RUST_PATH = /(^|[\s/'"])(target|\.cargo)(\/|$|[\s'"])/m;
const classes = JSON.parse(read(".github/event-classes.json"));

function saveExpression(step) {
  const inputs = step.with ?? {};
  if (step.uses?.startsWith(SETUP))
    return inputs.cache === false || inputs.cache === "false"
      ? undefined
      : (inputs["cache-save-if"] ?? true);
  if (step.uses?.startsWith(RUST_CACHE)) return inputs["save-if"] ?? true;
  if (CACHE_SAVE.test(step.uses ?? "") && RUST_PATH.test(String(inputs.path ?? ""))) return true;
  return undefined;
}

function triggers(entry, on) {
  const events = typeof on === "string" ? [on] : Array.isArray(on) ? on : Object.keys(on);
  if (!events.includes(entry.event)) return false;
  const spec = !Array.isArray(on) && typeof on === "object" ? on[entry.event] : null;
  if (entry.event !== "push" || !spec) return true;
  const matches = (patterns, value) =>
    patterns.some((pattern) =>
      new RegExp(`^${pattern.replace(/\*\*/g, ".*").replace(/\*/g, "[^/]*")}$`).test(value),
    );
  if (entry.tag) return Array.isArray(spec.tags) && matches(spec.tags, entry.tag);
  return !spec.branches || matches(spec.branches, entry.branch);
}

function contextFor(entry, matrix) {
  return {
    github: {
      event_name: entry.event,
      ref:
        entry.tag !== undefined
          ? `refs/tags/${entry.tag}`
          : `refs/heads/${entry.branch ?? "feature"}`,
      event: { action: entry.action ?? "", inputs: entry.inputs ?? {} },
      repository: "nickderobertis/onetaskgraph",
    },
    matrix,
    runner: { os: "Linux" },
    needs: {},
    env: {},
    vars: {},
    secrets: {},
    inputs: {},
    steps: {},
  };
}

// Reads no step output: a condition on one is taken as true, the answer that can only report more.
function holds(expression, context) {
  if (expression === undefined) return true;
  if (typeof expression !== "string") return expression !== false;
  if (!expression.includes("${{") && !/[=!&|()]/.test(expression)) return expression !== "false";
  const outputs = {};
  for (const [, id, name] of expression.matchAll(/steps\.([\w-]+)\.outputs\.([\w-]+)/g))
    outputs[id] = { outputs: { ...outputs[id]?.outputs, [name]: "true" } };
  const value = evaluate(expression, { ...context, steps: outputs });
  return value !== false && value !== "" && value !== "false" && value !== 0 && value !== null;
}

// Every workflow/job/step/class that would save a Rust build cache on something other than main.
function savers(definitions, inventory) {
  const found = [];
  for (const [file, workflow] of Object.entries(definitions)) {
    for (const [id, job] of Object.entries(workflow.jobs ?? {})) {
      const legs = job.strategy?.matrix?.include ?? [{}];
      for (const [index, step] of (job.steps ?? []).entries()) {
        const save = saveExpression(step);
        if (save === undefined) continue;
        for (const entry of inventory) {
          if (entry.event === "push" && entry.branch === "main") continue;
          if (!triggers(entry, workflow.on)) continue;
          for (const matrix of legs) {
            const context = contextFor(entry, matrix);
            if (holds(step.if, context) && holds(save, context))
              found.push(
                `${file}: job ${id}, step ${step.name ?? step.uses ?? index}, ${entry.id}`,
              );
          }
        }
      }
    }
  }
  return [...new Set(found)];
}

test("no Rust build cache step saves under any event class but a push to main", () => {
  assert.deepEqual(savers(workflows(), classes), []);
});

test("every Rust build cache step a tag, release or pull request reaches is found and restores only", () => {
  const seen = new Set();
  for (const [file, workflow] of Object.entries(workflows()))
    for (const [id, job] of Object.entries(workflow.jobs ?? {}))
      for (const step of job.steps ?? [])
        if (saveExpression(step) !== undefined) seen.add(`${file}:${id}`);
  for (const file of ["ci.yml", "release.yml", "release-plz.yml", "rust-cache-warm.yml"])
    assert.ok(
      [...seen].some((entry) => entry.startsWith(`${file}:`)),
      `${file} has a Rust build cache step`,
    );
});

const fixture = (body) => ({ "fixture.yml": parse(body) });

test("a Rust cache step that saves on a tag push is named", () => {
  assert.deepEqual(
    savers(
      fixture(`
on: { push: { tags: ["v*"], branches: [main] } }
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions-rust-lang/setup-rust-toolchain@v1
`),
      classes,
    ),
    ["fixture.yml: job build, step actions-rust-lang/setup-rust-toolchain@v1, release-tag"],
  );
});

test("a save condition is evaluated: main alone passes, a condition that admits a branch does not", () => {
  const pass = fixture(`
on: { push: { branches: [main, "cache-promote/**"] } }
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions-rust-lang/setup-rust-toolchain@v1
        with:
          cache-save-if: \${{ github.ref == 'refs/heads/main' }}
`);
  assert.deepEqual(savers(pass, classes), []);
  const fail = fixture(`
on: { push: { branches: [main, "cache-promote/**"] } }
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: Swatinem/rust-cache@v2
        with:
          save-if: \${{ startsWith(github.ref, 'refs/heads/') }}
`);
  assert.deepEqual(savers(fail, classes), [
    "fixture.yml: job build, step Swatinem/rust-cache@v2, cache-promotion",
  ]);
});

test("a plain actions/cache of target or the registry counts; one of something else does not", () => {
  const hit = fixture(`
on: pull_request
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/cache/save@v6
        with: { path: "target", key: k }
      - uses: actions/cache/save@v6
        with: { path: ".nx/cache", key: k }
`);
  assert.equal(savers(hit, classes).length, 3); // opened, synchronize, reopened; the one step
  assert.ok(savers(hit, classes).every((row) => row.includes("actions/cache/save@v6")));
});
