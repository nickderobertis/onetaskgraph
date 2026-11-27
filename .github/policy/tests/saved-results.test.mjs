import "../../test-setup.mjs";
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
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

test("test-linux measures the pool maintenance on a release pull request alone, before its one upload", () => {
  const { steps } = ci().jobs["test-linux"];
  const [upload] = uploads({ steps });
  const measuring = steps.find((step) => step.id === "pool-maintenance");
  assert.match(measuring.run, /exec just pool-maintenance-budget'$/);
  assert.equal(evaluate(measuring.if, context("pull_request", "release-plz-2026-11-07")), true);
  assert.equal(evaluate(measuring.if, context("pull_request", "feature")), false);
  assert.ok(steps.indexOf(upload) > steps.indexOf(measuring), "the upload follows what it uploads");
  const testing = steps.findIndex((step) => step.run === "just ci-test-linux");
  assert.ok(
    steps.indexOf(measuring) > testing,
    "it prunes a target directory the tests are done with",
  );
  assert.match(
    readFileSync(new URL("justfile", root), "utf8"),
    /onebudgetspec check pool-maintenance\/budgets\.yaml --label host --save-results target\/budget-results\/pool-maintenance-budget-results\.json/,
  );
});

test("lint measures no pool maintenance and uploads no saved results", () => {
  const { steps } = ci().jobs.lint;
  assert.deepEqual(uploads({ steps }), []);
  assert.equal(
    steps.some((step) => String(step.run ?? "").includes("pool-maintenance")),
    false,
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

// A cancel stops a step's shell and leaves what it started running, so the stop has to end the
// measurement's whole group before the cleanup builds beside it.
const stopping = () => {
  const { steps } = ci().jobs["test-linux"];
  const measuring = steps.findIndex((step) => step.id === "pool-maintenance");
  const stop = steps.findIndex(
    (step) => step.name === "Stop the cancelled pool-maintenance measurement",
  );
  return { steps, measuring, stop };
};

test("a cancelled measurement is stopped before any cleanup step builds, and only then", () => {
  const { steps, measuring, stop } = stopping();
  assert.ok(stop > measuring, "the stop follows the measurement");
  const builds = steps.findIndex((step) => String(step.run ?? "").startsWith("cargo "));
  assert.ok(builds > measuring, "a cleanup build follows the measurement");
  assert.ok(stop < builds, "the stop precedes every cleanup that builds into target/");
  const [upload] = uploads({ steps });
  assert.ok(stop < steps.indexOf(upload), "the stop precedes the upload of what it removes");
  const at = (job, step) => ({
    ...context("pull_request", "release-plz-2026-11-07"),
    needs: { job: { result: job } },
    steps: { "pool-maintenance": { conclusion: step } },
  });
  assert.equal(evaluate(steps[stop].if, at("cancelled", "cancelled")), true);
  assert.equal(evaluate(steps[stop].if, at("cancelled", "success")), false);
  assert.equal(evaluate(steps[stop].if, at("cancelled", "skipped")), false);
  assert.equal(evaluate(steps[stop].if, at("success", "success")), false);
});

const alive = (pid) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
};
const until = async (condition, seconds, what) => {
  const deadline = Date.now() + seconds * 1000;
  while (!condition()) {
    assert.ok(Date.now() < deadline, what);
    await new Promise((settle) => setTimeout(settle, 100));
  }
};

test("the stop step ends a measurement that outlived its step's shell, and nothing it writes is uploaded", async () => {
  const { steps, measuring, stop } = stopping();
  const scratch = mkdtempSync(
    join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir(), "pool-stop-"),
  );
  try {
    const bin = join(scratch, "bin");
    const runnerTemp = join(scratch, "runner-temp");
    mkdirSync(bin);
    mkdirSync(runnerTemp);
    mkdirSync(join(scratch, "pool-maintenance"));
    copyFileSync(
      new URL("pool-maintenance/stop.sh", root),
      join(scratch, "pool-maintenance/stop.sh"),
    );
    const saved = join(scratch, "target/budget-results/pool-maintenance-budget-results.json");
    const writer = join(scratch, "writer.pid");
    // A stand-in for the measurement: a member of its group that writes the saved result late,
    // as onebudgetspec did minutes after the cancel, and a leader that waits on it.
    writeFileSync(
      join(bin, "just"),
      `#!/usr/bin/env bash
[ "$1" = pool-maintenance-budget ] || exit 64
( sleep 4; mkdir -p target/budget-results; echo '{"verdict":"error"}' > target/budget-results/pool-maintenance-budget-results.json ) &
echo "$!" > ${JSON.stringify(writer)}
wait
`,
    );
    chmodSync(join(bin, "just"), 0o755);
    const env = { ...process.env, PATH: `${bin}:${process.env.PATH}`, RUNNER_TEMP: runnerTemp };
    const shell = spawn("bash", ["-e", "-c", steps[measuring].run], {
      cwd: scratch,
      env,
      stdio: "ignore",
    });
    await until(
      () => existsSync(writer) && readFileSync(writer, "utf8").trim() !== "",
      10,
      "the measurement started",
    );
    const member = Number(readFileSync(writer, "utf8"));
    // The runner stops the step's shell; what it started runs on, which is the defect.
    shell.kill("SIGKILL");
    await new Promise((settle) => shell.once("exit", settle));
    assert.ok(alive(member), "the measurement outlives its step's shell, as on the hosted runner");
    const stopped = spawnSync("bash", ["-e", "-c", steps[stop].run], {
      cwd: scratch,
      env,
      encoding: "utf8",
    });
    assert.equal(stopped.status, 0, stopped.stderr);
    assert.equal(alive(member), false, "no member of the measurement's group is left");
    await new Promise((settle) => setTimeout(settle, 5000));
    assert.equal(
      existsSync(saved),
      false,
      "nothing the stopped measurement would have saved is left to upload",
    );
    assert.match(stopped.stderr, /stopped the cancelled measurement/);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("the stop step with no measurement to stop removes a stale result and signals nothing", () => {
  const { steps, stop } = stopping();
  const scratch = mkdtempSync(
    join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir(), "pool-stop-"),
  );
  try {
    mkdirSync(join(scratch, "pool-maintenance"));
    copyFileSync(
      new URL("pool-maintenance/stop.sh", root),
      join(scratch, "pool-maintenance/stop.sh"),
    );
    mkdirSync(join(scratch, "target/budget-results"), { recursive: true });
    const saved = join(scratch, "target/budget-results/pool-maintenance-budget-results.json");
    writeFileSync(saved, "{}");
    const env = { ...process.env, RUNNER_TEMP: join(scratch, "absent") };
    const stopped = spawnSync("bash", ["-e", "-c", steps[stop].run], {
      cwd: scratch,
      env,
      encoding: "utf8",
    });
    assert.equal(stopped.status, 0, stopped.stderr);
    assert.equal(stopped.stderr, "");
    assert.equal(existsSync(saved), false);
    // A session file that names no group is refused rather than read as a process id.
    mkdirSync(join(scratch, "absent"));
    writeFileSync(join(scratch, "absent/pool-maintenance.session"), "1\n");
    const refused = spawnSync("bash", ["-e", "-c", steps[stop].run], {
      cwd: scratch,
      env,
      encoding: "utf8",
    });
    assert.equal(refused.status, 1);
    assert.match(refused.stderr, /not a session's process group id/);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});
