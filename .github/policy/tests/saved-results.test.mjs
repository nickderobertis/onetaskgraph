import "../../test-setup.mjs";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { parse } from "yaml";
import { evaluate, graph, read, root } from "../../workflow-graph.mjs";

// What ci.yml does with the results its budgets targets save, as the run graph resolves the real
// workflow file. A journey carrying a saved figure from its target to its row is report-job's
// (.github/reporting/test_report_job.py, SavedResults); this holds what that one proves for one
// configuration across every event and platform.

const ci = () => parse(read(".github/workflows/ci.yml"));
const project = (path) => JSON.parse(read(path));
// The name ends so because .github/nx-policy holds a cached output to a report-shaped name.
const results = (name) => `target/budget-results/${name}-budget-results.json`;
const uploads = (job) =>
  job.steps.filter((step) => String(step.with?.name ?? "").startsWith("budget-check-results-"));
const repository = "nickderobertis/onetaskgraph";
const pullRequest = (headRef) => ({
  event: "pull_request",
  action: "synchronize",
  headRef,
  fork: false,
});
const running = (scenario) =>
  graph(scenario, { macosRequired: true, reason: "a change" })
    .filter((run) => run.workflow === "ci.yml")
    .flatMap((run) => run.rows.map((row) => row.name));
const context = (eventName, headRef) => ({
  github: {
    event_name: eventName,
    head_ref: headRef,
    repository,
    event: { pull_request: { head: { repo: { fork: false } } } },
  },
  needs: {},
});

// Each target test-linux runs, by what it runs and the file it saves; `cached` says Nx must restore it.
const SAVING = {
  "github-workflow-policy": { file: ".github/policy/project.json", cached: false },
  "report-job": { file: ".github/reporting/project.json", cached: false },
  "onetaskgraph-github-projects-e2e": {
    file: "crates/onetaskgraph-github-projects-e2e/project.json",
    cached: true,
  },
  "onetaskgraph-linear-e2e": {
    file: "crates/onetaskgraph-linear-e2e/project.json",
    cached: true,
  },
  "onetaskgraph-live-janitor": {
    file: "crates/onetaskgraph-live-janitor/project.json",
    cached: true,
  },
};

test("a pull request runs the five jobs and a push to main runs budgets alone", () => {
  for (const branch of ["feature", "release-plz-2026-11-07"])
    assert.deepEqual(running(pullRequest(branch)), [
      "lint",
      "test-linux",
      "test-windows",
      "test-macos",
      "budgets",
    ]);
  assert.deepEqual(
    Object.entries(ci().jobs)
      .filter(([, job]) => evaluate(job.if ?? "true", context("push")))
      .map(([name]) => name),
    ["budgets"],
  );
});

test("test-linux uploads what its targets saved however the job ended, on every pull request", () => {
  const [upload, ...more] = uploads(ci().jobs["test-linux"]);
  assert.deepEqual(more, []);
  assert.equal(upload.with.name, "budget-check-results-test-linux");
  assert.equal(upload.with.path, "target/budget-results/");
  assert.equal(upload.if, "always()");
  assert.equal(evaluate(upload.if, context("pull_request", "feature")), true);
});

test("lint uploads its one step's results on a release pull request and on no other", () => {
  const { steps } = ci().jobs.lint;
  const [upload, ...more] = uploads({ steps });
  assert.deepEqual(more, []);
  assert.equal(upload.with.name, "budget-check-results-lint");
  assert.equal(upload.with.path, "target/budget-results/");
  const measuring = steps.find((step) => step.run === "just pool-maintenance-budget");
  assert.equal(evaluate(measuring.if, context("pull_request", "release-plz-2026-11-07")), true);
  assert.equal(evaluate(measuring.if, context("pull_request", "feature")), false);
  assert.equal(evaluate(upload.if, context("pull_request", "release-plz-2026-11-07")), true);
  assert.equal(evaluate(upload.if, context("pull_request", "feature")), false);
  assert.ok(steps.indexOf(upload) > steps.indexOf(measuring), "the upload follows what it uploads");
  assert.match(
    readFileSync(new URL("justfile", root), "utf8"),
    /onebudgetspec check pool-maintenance\/budgets\.yaml --label host --save-results target\/budget-results\/pool-maintenance-budget-results\.json/,
  );
});

test("the other platforms run the same targets and upload nothing, so a figure stays Linux's", () => {
  for (const name of ["test-windows", "test-macos"])
    assert.deepEqual(uploads(ci().jobs[name]), [], name);
});

test("every target test-linux runs saves its own file, and Nx restores it where Nx caches it", () => {
  const command = (target) => target.options.command;
  for (const [name, { file, cached }] of Object.entries(SAVING)) {
    const definition = project(file);
    assert.equal(definition.name, name);
    const target = definition.targets.budgets;
    assert.equal(target.cache === true, cached, `${name}:budgets cache`);
    if (cached) assert.deepEqual(target.outputs, [`{workspaceRoot}/${results(name)}`]);
    // Two of the commands are small programs of their own, and they name it there.
    const programs = {
      "report-job": ".github/reporting/check_budgets.py",
      "onetaskgraph-github-projects-e2e": "crates/onetaskgraph-github-projects-e2e/lane_runner.py",
    };
    const text = programs[name] ? read(programs[name]) : command(target);
    assert.ok(text.includes(results(name)), `${name}:budgets saves ${results(name)}`);
    assert.match(text, /save-results/, name);
  }
});

test("the budgets job reads every saved artifact and leaves the labelled budgets out", () => {
  const { steps } = ci().jobs.budgets;
  const given = steps.find((step) => step.uses?.includes("onebudgetspec-github-action")).with;
  assert.equal(given["saved-results-artifacts"], "budget-check-results-*");
  assert.equal(given["omit-labels"], "not-measured-on-pr");
  assert.equal(given["enforcement-exclude-labels"], "release-tail,report-only");
});
