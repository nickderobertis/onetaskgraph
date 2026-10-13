import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { validateTelemetry } from "../telemetry.ts";
import { digest, graph, jobs, triggered } from "../workflow-graph.mjs";

function recording() {
  const scenario = {
    id: "boundary-proof",
    event: "pull_request",
    action: "opened",
    diff: "source",
    fork: false,
    vars: {},
  };
  const runs = graph(scenario, { macosRequired: true, reason: "ordinary PR" }).map((run) => ({
    ...run,
    macJobs: run.rows.filter((row) => row.macos).length,
  }));
  return {
    schemaVersion: 1,
    sourceDigest: digest(),
    scenarios: [{ ...scenario, runs }],
    lifecycles: [
      {
        diff: scenario.diff,
        fork: false,
        optional: 0,
        runs,
        macRuns: runs.filter((run) => run.macJobs > 0).length,
      },
    ],
  };
}
test("telemetry boundary refuses malformed shapes, invalid measurements and inconsistent rows", () => {
  const current = recording();
  validateTelemetry(current);
  for (const invalid of [
    null,
    {},
    { ...current, scenarios: {} },
    { ...current, lifecycles: [] },
    { ...current, schemaVersion: "1" },
  ])
    assert.throws(() => validateTelemetry(invalid));
  for (const value of [NaN, Infinity, -1, 0.5, "1", null]) {
    const corrupted = structuredClone(current);
    corrupted.scenarios[0].runs[0].macJobs = value;
    assert.throws(() => validateTelemetry(corrupted), /measurement/);
  }
  const hiddenMac = structuredClone(current);
  const macRun = hiddenMac.scenarios[0].runs.find((run) => run.rows.some((row) => row.macos));
  macRun.rows.find((row) => row.macos).macos = false;
  macRun.macJobs = macRun.rows.filter((row) => row.macos).length;
  assert.throws(() => validateTelemetry(hiddenMac), /classification/);
  const empty = structuredClone(current);
  empty.scenarios[0].runs = [];
  assert.throws(() => validateTelemetry(empty), /no workflow measurements/);
  const absentLifecycle = structuredClone(current);
  absentLifecycle.lifecycles[0].runs = [];
  absentLifecycle.lifecycles[0].macRuns = 0;
  assert.throws(() => validateTelemetry(absentLifecycle), /no lifecycle measurements/);
  const badPhase = structuredClone(current);
  badPhase.lifecycles[0].runs[0].phase = {};
  assert.throws(() => validateTelemetry(badPhase), /string/);
  const corrupted = structuredClone(current);
  corrupted.scenarios[0].runs[0].rows[0].runner = [];
  assert.throws(() => validateTelemetry(corrupted), /runner/);
  assert.throws(
    () => jobs({ jobs: { unknown: { "runs-on": "unclassified-latest" } } }, { event: "push" }, {}),
    /Unresolved runner OS/,
  );
});

test("real budget reporter rejects missing and stale recordings without rerunning scenarios", () => {
  const scratch = mkdtempSync(
    join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir(), "budget-boundary-"),
  );
  try {
    const reporter = fileURLToPath(new URL("../macos-budget.mjs", import.meta.url));
    const path = join(scratch, "recording.json");
    const command = [reporter, "--metric", "jobs-per-run", "--telemetry", path];
    assert.notEqual(spawnSync(process.execPath, command, { encoding: "utf8" }).status, 0);
    const current = recording();
    current.sourceDigest = "0".repeat(64);
    writeFileSync(path, JSON.stringify(current));
    const stale = spawnSync(process.execPath, command, { encoding: "utf8" });
    assert.notEqual(stale.status, 0);
    assert.match(stale.stderr, /stale/);
    writeFileSync(path, "{bad");
    assert.notEqual(spawnSync(process.execPath, command, { encoding: "utf8" }).status, 0);
    execFileSync(process.execPath, ["--check", reporter]);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("real reporters write both measurements through the SDK result boundary", () => {
  const scratch = mkdtempSync(
    join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir(), "reporter-proof-"),
  );
  try {
    const current = recording();
    const path = join(scratch, "recording.json");
    writeFileSync(path, JSON.stringify(current));
    for (const metric of ["jobs-per-run", "runs-per-change"]) {
      const resultPath = join(scratch, `${metric}.result`);
      execFileSync(
        process.execPath,
        [
          fileURLToPath(new URL("../macos-budget.mjs", import.meta.url)),
          "--metric",
          metric,
          "--telemetry",
          path,
        ],
        { env: { ...process.env, ONEBUDGETSPEC_RESULT: resultPath } },
      );
      const result = JSON.parse(readFileSync(resultPath, "utf8"));
      const expected =
        metric === "jobs-per-run"
          ? Math.max(...current.scenarios.flatMap((s) => s.runs.map((r) => r.macJobs)))
          : Math.max(...current.lifecycles.map((l) => l.macRuns));
      assert.equal(result.value, expected);
      assert.ok(JSON.parse(result.detail).runs.some((r) => r.workflow === "ci.yml"));
    }
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("trigger filters discriminate event types, branches and tags and refuse unsupported filters", () => {
  const workflow = {
    jobs: { proof: { "runs-on": "ubuntu-latest" } },
    on: {
      push: { branches: ["main"] },
      release: { types: ["published"] },
      workflow_run: { workflows: ["ci"] },
    },
  };
  assert.equal(triggered(workflow, { event: "push", branch: "main" }), true);
  assert.equal(triggered(workflow, { event: "push", branch: "other" }), false);
  assert.equal(triggered(workflow, { event: "push", tag: "v1.2.3" }), false);
  assert.equal(triggered(workflow, { event: "release", action: "created" }), false);
  assert.equal(triggered(workflow, { event: "release", action: "published" }), true);
  assert.equal(triggered(workflow, { event: "workflow_run", workflow: "other" }), false);
  for (const on of [null, 42, { push: [] }, { push: "bad" }, { schedule: [{}] }, { unknown: null }])
    assert.throws(
      () => triggered({ on, jobs: { proof: { "runs-on": "ubuntu-latest" } } }, { event: "push" }),
      /Unresolved/,
    );
  for (const filter of [
    { paths: ["src/**"] },
    { "branches-ignore": ["draft"] },
    { branches: ["release-*"] },
  ])
    assert.throws(
      () =>
        triggered(
          { jobs: { proof: { "runs-on": "ubuntu-latest" } }, on: { push: filter } },
          { event: "push", branch: "main" },
        ),
      /Unresolved/,
    );
});

test("workflow_run fan-out follows actual declarations and rejects cycles", () => {
  const declared = {
    "source.yml": {
      name: "source",
      on: { push: { branches: ["main"] } },
      jobs: { check: { "runs-on": "ubuntu-latest" } },
    },
    "smoke.yml": {
      name: "smoke",
      on: { workflow_run: { workflows: ["source"], types: ["completed"] } },
      jobs: { smoke: { "runs-on": "ubuntu-latest" } },
    },
  };
  assert.deepEqual(
    graph({ event: "push", branch: "main" }, { macosRequired: false }, declared).map((run) => [
      run.workflow,
      run.event,
    ]),
    [
      ["source.yml", "push"],
      ["smoke.yml", "workflow_run"],
    ],
  );
  declared["source.yml"].on.workflow_run = { workflows: ["smoke"] };
  assert.throws(
    () => graph({ event: "push", branch: "main" }, { macosRequired: false }, declared),
    /cycle/,
  );
});

test("local Bun parser rejects unsupported CLI arguments at its real entrypoint", () => {
  const helper = fileURLToPath(new URL("../semantic-json.mjs", import.meta.url));
  const input = readFileSync(new URL("../../bun.lock", import.meta.url), "utf8");
  for (const args of [["--unknown"], ["extra"], ["--parse-only", "--parse-only"]]) {
    const result = spawnSync(process.execPath, [helper, ...args], { input, encoding: "utf8" });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /usage/);
  }
  const accepted = spawnSync(process.execPath, [helper, "--parse-only"], {
    input,
    encoding: "utf8",
  });
  assert.equal(accepted.status, 0, accepted.stderr);
  assert.equal(JSON.parse(accepted.stdout).workspaces["sdks/typescript"].name, "@onetaskgraph/sdk");
});
