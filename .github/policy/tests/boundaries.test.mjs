import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import {
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
  readdirSync,
  cpSync,
  mkdirSync,
  symlinkSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { stripVTControlCharacters } from "node:util";
import { validateTelemetry } from "../../telemetry.ts";
import {
  digest,
  evaluate,
  graph,
  jobs,
  matrices,
  triggered,
  validateEventInventory,
  read,
  root,
  workflows,
} from "../../workflow-graph.mjs";

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
        runs: structuredClone(runs),
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
  const wrongJobs = structuredClone(current);
  wrongJobs.scenarios[0].runs[0].macJobs += 1;
  assert.throws(() => validateTelemetry(wrongJobs), /job total/);
  const wrongRuns = structuredClone(current);
  wrongRuns.lifecycles[0].macRuns += 1;
  assert.throws(() => validateTelemetry(wrongRuns), /lifecycle total/);
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
    const lifecycleRun = current.lifecycles[0].runs.find((run) => run.macJobs > 0);
    const macRow = lifecycleRun.rows.find((row) => row.macos);
    lifecycleRun.rows.push({
      ...macRow,
      job: `${macRow.job}-additional`,
      name: "additional Mac proof",
    });
    lifecycleRun.macJobs += 1;
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
          ? Math.max(...current.lifecycles.flatMap((l) => l.runs.map((r) => r.macJobs)))
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
  const barePR = { on: { pull_request: null }, jobs: { proof: { "runs-on": "ubuntu-latest" } } };
  assert.equal(triggered(barePR, { event: "pull_request", action: "opened" }), true);
  assert.equal(triggered(barePR, { event: "pull_request", action: "closed" }), false);
  assert.equal(triggered(workflow, { event: "push", branch: "main" }), true);
  assert.equal(triggered(workflow, { event: "push", branch: "other" }), false);
  assert.equal(triggered(workflow, { event: "push", tag: "v1.2.3" }), false);
  assert.equal(triggered(workflow, { event: "release", action: "created" }), false);
  assert.equal(triggered(workflow, { event: "release", action: "published" }), true);
  assert.equal(triggered(workflow, { event: "workflow_run", workflow: "other" }), false);
  const combined = { ...workflow, on: { push: { branches: ["main"], tags: ["v1.2.4"] } } };
  for (const [scenario, expected] of [
    [{ event: "push", branch: "main" }, true],
    [{ event: "push", tag: "v1.2.4" }, true],
    [{ event: "push", branch: "other" }, false],
    [{ event: "push", tag: "v9.0.0" }, false],
  ])
    assert.equal(triggered(combined, scenario), expected);

  for (const on of [null, 42, { push: [] }, { push: "bad" }, { schedule: [{}] }, { unknown: null }])
    assert.throws(
      () => triggered({ on, jobs: { proof: { "runs-on": "ubuntu-latest" } } }, { event: "push" }),
      /Unresolved/,
    );
  for (const key of ["types", "branches", "tags", "workflows"])
    assert.throws(
      () =>
        triggered(
          { jobs: { proof: { "runs-on": "ubuntu-latest" } }, on: { push: { [key]: "main" } } },
          { event: "push" },
        ),
      /Unresolved/,
    );
  for (const key of ["include", "exclude"])
    for (const value of ["bad", [null], ["bad"], [[]]])
      assert.throws(() => matrices({ os: ["ubuntu-latest"], [key]: value }, {}), /Unresolved/);
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
      run.triggeredBy || null,
    ]),
    [
      ["source.yml", "push", null],
      ["smoke.yml", "workflow_run", "source"],
    ],
  );
  declared["source.yml"].on.workflow_run = { workflows: ["smoke"] };
  assert.throws(
    () => graph({ event: "push", branch: "main" }, { macosRequired: false }, declared),
    /cycle/,
  );
});

test("local Bun parser rejects unsupported CLI arguments at its real entrypoint", () => {
  const helper = fileURLToPath(new URL("../../semantic-json.mjs", import.meta.url));
  const input = readFileSync(new URL("../../../bun.lock", import.meta.url), "utf8");
  for (const args of [["--unknown"], ["extra"], ["--parse-only", "--parse-only"]]) {
    const result = spawnSync(process.execPath, [helper, ...args], { input, encoding: "utf8" });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /usage/);
  }
  for (const bad of ["null", "42", "[]", "{}", '{"workspaces":[]}'])
    assert.notEqual(
      spawnSync(process.execPath, [helper], { input: bad, encoding: "utf8" }).status,
      0,
    );
  const accepted = spawnSync(process.execPath, [helper, "--parse-only"], {
    input,
    encoding: "utf8",
  });
  assert.equal(accepted.status, 0, accepted.stderr);
  assert.equal(JSON.parse(accepted.stdout).workspaces["sdks/typescript"].name, "@onetaskgraph/sdk");
});

test("classifier and carriers enforce invocation and actual filesystem IO failure contracts", () => {
  const scratch = mkdtempSync(
    join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir(), "cli-io-"),
  );
  try {
    const classifier = fileURLToPath(new URL("../../macos-policy.mjs", import.meta.url));
    const carrier = fileURLToPath(new URL("../../macos-result.mjs", import.meta.url));
    const event = join(scratch, "event.json");
    writeFileSync(event, JSON.stringify({ event_name: "push" }));
    for (const args of [[], ["other"], ["classify", "--unknown", event]])
      assert.notEqual(
        spawnSync(process.execPath, [classifier, ...args], { encoding: "utf8" }).status,
        0,
      );
    const outputFailure = spawnSync(
      process.execPath,
      [classifier, "classify", "--event", event, "--base", "missing", "--head", "missing"],
      {
        env: { ...process.env, GITHUB_EVENT_NAME: "push", GITHUB_OUTPUT: scratch },
        encoding: "utf8",
      },
    );
    assert.notEqual(outputFailure.status, 0);
    const needs = join(scratch, "needs.json");
    for (const path of [join(scratch, "missing.json"), scratch])
      assert.notEqual(spawnSync(process.execPath, [carrier, path], { encoding: "utf8" }).status, 0);
    writeFileSync(needs, "{bad");
    assert.notEqual(spawnSync(process.execPath, [carrier, needs], { encoding: "utf8" }).status, 0);
    for (const reason of [undefined, null, {}, "", " \n"]) {
      writeFileSync(
        needs,
        JSON.stringify({
          "macos-lanes": { result: "success", outputs: { macosRequired: "false", reason } },
          macos: { result: "skipped" },
          "test-macos": { result: "skipped" },
        }),
      );
      assert.notEqual(
        spawnSync(process.execPath, [carrier, needs], { encoding: "utf8" }).status,
        0,
      );
    }
    writeFileSync(
      needs,
      JSON.stringify({
        "macos-lanes": {
          result: "success",
          outputs: { macosRequired: "false", reason: "authorized semantic release" },
        },
        macos: { result: "skipped" },
        "test-macos": { result: "skipped" },
      }),
    );
    assert.notEqual(
      spawnSync(process.execPath, [carrier, needs, "extra"], { encoding: "utf8" }).status,
      0,
    );
    assert.equal(spawnSync(process.execPath, [carrier, needs], { encoding: "utf8" }).status, 0);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("reporter rejects invalid invocations and an unwritable SDK result destination", () => {
  const scratch = mkdtempSync(
    join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir(), "reporter-io-"),
  );
  try {
    const reporter = fileURLToPath(new URL("../macos-budget.mjs", import.meta.url));
    for (const args of [
      [],
      ["--metric", "other", "--telemetry", "missing"],
      ["--wrong", "jobs-per-run", "--telemetry", "missing"],
    ])
      assert.notEqual(
        spawnSync(process.execPath, [reporter, ...args], { encoding: "utf8" }).status,
        0,
      );
    const path = join(scratch, "telemetry.json");
    writeFileSync(path, JSON.stringify(recording()));
    const result = spawnSync(
      process.execPath,
      [reporter, "--metric", "jobs-per-run", "--telemetry", path],
      { env: { ...process.env, ONEBUDGETSPEC_RESULT: scratch }, encoding: "utf8" },
    );
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /next:/);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("event workload rejects new workflow triggers, actions and diagnostic inputs", () => {
  const inventory = JSON.parse(read(".github/event-classes.json"));
  validateEventInventory(inventory);
  for (const mutation of [
    (workflow) => {
      workflow.on.push = { branches: ["unmeasured"] };
    },
    (workflow) => {
      workflow.on.push = { tags: ["v99.0.0"] };
    },
    (workflow) => {
      workflow.on.workflow_run = { workflows: ["unknown-upstream"] };
    },
    (workflow) => {
      workflow.on.issue_comment = null;
    },
    (workflow) => {
      workflow.on.pull_request = { types: ["edited"] };
    },
    (workflow) => {
      workflow.on.workflow_dispatch.inputs.extra = { type: "boolean", default: false };
    },
    (workflow) => {
      workflow.on.workflow_dispatch.inputs.macos.type = "string";
    },
  ]) {
    const definitions = workflows();
    mutation(definitions["ci.yml"]);
    assert.throws(() => validateEventInventory(inventory, definitions), /Unmeasured workflow/);
  }
});

test("workflow boundary accepts declared shorthand, matrix and runner forms and refuses malformed ones", () => {
  const runnerJob = { proof: { "runs-on": "ubuntu-latest" } };
  for (const on of ["push", ["push", "release"]]) {
    assert.equal(triggered({ on, jobs: runnerJob }, { event: "push" }), true);
    assert.equal(triggered({ on, jobs: runnerJob }, { event: "schedule" }), false);
  }
  const tagged = { on: { push: { tags: ["v1.2.4"] } }, jobs: runnerJob };
  assert.equal(triggered(tagged, { event: "push", tag: "v1.2.4" }), true);
  assert.equal(triggered(tagged, { event: "push", tag: "v1.2.5" }), false);
  for (const invalid of [false, 0, "", "invalid", [], { os: "macos-latest" }])
    assert.throws(() => matrices(invalid, {}), /Unresolved matrix/);
  assert.deepEqual(matrices({ include: [{ os: "macos-latest" }] }, {}), [{ os: "macos-latest" }]);
  for (const invalid of [null, [], {}])
    assert.throws(
      () => triggered({ on: "push", jobs: { proof: invalid } }, { event: "push" }),
      /Unresolved workflow job/,
    );
  const scenario = { event: "pull_request", action: "opened" };
  const policy = { macosRequired: true, reason: "ordinary" };
  assert.equal(
    jobs({ jobs: { proof: { "runs-on": { labels: ["macos-latest"] } } } }, scenario, policy)[0]
      .macos,
    true,
  );
  assert.throws(
    () => jobs({ jobs: { proof: { "runs-on": ["self-hosted", "custom"] } } }, scenario, policy),
    /Unknown self-hosted OS/,
  );
  for (const result of ["success", "failure", "cancelled", "skipped"]) {
    const context = { needs: { prerequisite: { result } } };
    assert.equal(evaluate("success()", context), result === "success");
    assert.equal(evaluate("failure()", context), result === "failure");
    assert.equal(evaluate("cancelled()", context), result === "cancelled");
    assert.equal(evaluate("always()", context), true);
  }
});

test("event inventory validates serialized identities, ownership and inputs before routing", () => {
  for (const inventory of [
    null,
    {},
    [],
    [null],
    [{ id: "bad!", event: "push" }],
    [{ id: "entry", event: 0 }],
    [{ id: "entry", event: "push", action: false }],
    [{ id: "entry", event: "push", fork: "false" }],
    [{ id: "entry", event: "workflow_dispatch", inputs: [] }],
    [{ id: "entry", event: "workflow_dispatch", inputs: { macos: "true" } }],
    [
      { id: "entry", event: "push" },
      { id: "entry", event: "push" },
    ],
  ])
    assert.throws(() => validateEventInventory(inventory), /Invalid event inventory/);
});

test("real reporter invalidates previously valid telemetry when workflow source changes", () => {
  const scratch = mkdtempSync(
    join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir(), "freshness-proof-"),
  );
  try {
    cpSync(fileURLToPath(new URL(".github/", root)), join(scratch, ".github"), {
      recursive: true,
      filter: (path) => !/[\\/]node_modules(?:[\\/]|$)|[\\/]\.telemetry(?:[\\/]|$)/.test(path),
    });
    symlinkSync(
      fileURLToPath(new URL(".github/node_modules", root)),
      join(scratch, ".github/node_modules"),
      process.platform === "win32" ? "junction" : "dir",
    );
    mkdirSync(join(scratch, "scripts"));
    for (const path of [
      "release-targets.toml",
      "screencomp.toml",
      "scripts/product_versions.py",
      "scripts/release_products.py",
    ])
      writeFileSync(join(scratch, path), read(path));
    const telemetry = join(scratch, "recording.json");
    writeFileSync(telemetry, JSON.stringify(recording()));
    const command = [
      join(scratch, ".github/policy/macos-budget.mjs"),
      "--metric",
      "jobs-per-run",
      "--telemetry",
      telemetry,
    ];
    execFileSync(process.execPath, command, { cwd: scratch });
    const source = join(scratch, ".github/workflows/ci.yml");
    writeFileSync(source, `${readFileSync(source, "utf8")}\n# changed source\n`);
    const stale = spawnSync(process.execPath, command, { cwd: scratch, encoding: "utf8" });
    assert.notEqual(stale.status, 0);
    assert.match(stale.stderr, /stale/);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("workflow parser refuses empty trigger and scheduling declarations and malformed consumed job fields", () => {
  for (const on of [[], {}, [null], [""]])
    assert.throws(
      () => triggered({ on, jobs: { proof: { "runs-on": "ubuntu-latest" } } }, { event: "push" }),
      /Unresolved workflow/,
    );
  for (const matrix of [{}, { os: [] }])
    assert.throws(() => matrices(matrix, {}), /Unresolved matrix/);
  for (const fields of [
    { needs: {} },
    { if: [] },
    { outputs: [] },
    { with: false },
    { env: [] },
    { steps: {} },
    { steps: [{ run: 0 }] },
    { steps: [{ env: [] }] },
    { strategy: [] },
    { uses: 0 },
  ])
    assert.throws(
      () =>
        triggered(
          { on: "push", jobs: { proof: { "runs-on": "ubuntu-latest", ...fields } } },
          { event: "push" },
        ),
      /Unresolved workflow job/,
    );
});

test("schedule, display-name and release target boundaries refuse unresolved telemetry", () => {
  for (const cron of ["", "99 * * * *", "0 0 * *", "0 0 32 * *", "0 0 * 13 *"])
    assert.throws(
      () =>
        triggered(
          { on: { schedule: [{ cron }] }, jobs: { proof: { "runs-on": "ubuntu-latest" } } },
          { event: "schedule" },
        ),
      /Unresolved workflow trigger/,
    );
  for (const name of [{}, []])
    assert.throws(
      () =>
        triggered(
          { on: "push", jobs: { proof: { name, "runs-on": "ubuntu-latest" } } },
          { event: "push" },
        ),
      /Unresolved workflow job/,
    );
  for (const TARGETS of [
    true,
    1,
    "",
    "invalid",
    "aarch64-apple-darwin=invalid",
    "aarch64-apple-darwin=darwin-arm64=extra",
  ])
    assert.throws(
      () =>
        jobs(
          { jobs: { proof: { "runs-on": "macos-latest", env: { TARGETS } } } },
          { event: "release" },
          { macosRequired: false },
        ),
      /Unresolved release TARGETS/,
    );
});

test("arches subprocess cache distinguishes actual command bodies and resolved step environments", () => {
  const body = `printf 'matrix={"include":[{"arch":"x86_64","label":"%s"}]}\\n' "$TITLE" > "$GITHUB_OUTPUT"`;
  const workflow = (run, title) => ({
    jobs: {
      arches: { "runs-on": "ubuntu-latest", steps: [{ id: "read", run, env: { TITLE: title } }] },
      capture: {
        "runs-on": "ubuntu-latest",
        needs: ["arches"],
        strategy: { matrix: `\${{ fromJson(needs.arches.outputs.matrix) }}` },
        name: `\${{ matrix.label }}`,
      },
    },
  });
  const scenario = { event: "pull_request", action: "opened" };
  const policy = { macosRequired: true };
  const label = (run, title) =>
    jobs(workflow(run, title), scenario, policy).find((row) => row.job === "capture").matrix.label;
  assert.equal(label(body, "first"), "first");
  assert.equal(label(body, "second"), "second");
  assert.equal(label(body.replace('"$TITLE"', '"third"'), "second"), "third");
});

test("excluded matrices and disabled arches contribute no jobs and never execute skipped discovery", () => {
  const scenario = { event: "pull_request", action: "opened" };
  const policy = { macosRequired: true };
  assert.deepEqual(
    jobs(
      {
        jobs: {
          proof: {
            "runs-on": `\${{ matrix.os }}`,
            strategy: { matrix: { os: ["macos-latest"], exclude: [{ os: "macos-latest" }] } },
          },
        },
      },
      scenario,
      policy,
    ),
    [],
  );
  assert.deepEqual(
    jobs(
      {
        jobs: {
          arches: {
            "runs-on": "ubuntu-latest",
            if: "false",
            steps: [{ id: "read", run: "exit 89" }],
          },
          capture: { "runs-on": "ubuntu-latest", needs: ["arches"] },
        },
      },
      scenario,
      policy,
    ),
    [],
  );
  for (const inputs of [
    false,
    [],
    "",
    { enabled: false },
    { enabled: { type: "boolean", default: "false" } },
  ])
    assert.throws(
      () =>
        triggered(
          {
            on: { workflow_dispatch: { inputs } },
            jobs: { proof: { "runs-on": "ubuntu-latest" } },
          },
          { event: "workflow_dispatch" },
        ),
      /Unresolved workflow inputs/,
    );
});

test("workflow names are validated and status overrides preserve dependent-job scheduling after skips", () => {
  for (const name of ["", {}, []])
    assert.throws(
      () =>
        triggered(
          { name, on: "push", jobs: { proof: { "runs-on": "ubuntu-latest" } } },
          { event: "push" },
        ),
      /Unresolved workflow declaration/,
    );
  for (const condition of ["always()", "!cancelled()", "success()", "failure()", "cancelled()"]) {
    const rows = jobs(
      {
        jobs: {
          prerequisite: { "runs-on": "ubuntu-latest", if: "false" },
          report: { "runs-on": "ubuntu-latest", needs: ["prerequisite"], if: condition },
        },
      },
      { event: "pull_request", action: "opened" },
      { macosRequired: true },
    );
    assert.deepEqual(
      rows.map((row) => row.job),
      ["always()", "!cancelled()"].includes(condition) ? ["report"] : [],
    );
  }
});

test("failing architecture discovery propagates the real shell exit and removes its scratch directory", () => {
  const module = new URL("../../workflow-graph.mjs", import.meta.url).href;
  const { FORCE_COLOR, NO_COLOR, ...inherited } = process.env;
  // Hosted runners force colour, so Node's error dump carries ANSI codes
  // around the status; the assertion must read the same either way.
  for (const colour of [{ FORCE_COLOR: "1" }, { NO_COLOR: "1" }]) {
    const scratch = mkdtempSync(
      join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir(), "discovery-failure-"),
    );
    try {
      const result = spawnSync(
        process.execPath,
        [
          "--input-type=module",
          "-e",
          `import {jobs} from ${JSON.stringify(module)}; jobs({jobs:{arches:{"runs-on":"ubuntu-latest",steps:[{id:"read",run:"exit 37"}]}}},{event:"pull_request",action:"opened"},{macosRequired:true});`,
        ],
        {
          encoding: "utf8",
          env: { ...inherited, ...colour, ONEPIPELINE_NODE_SCRATCH_DIR: scratch },
        },
      );
      assert.notEqual(result.status, 0);
      assert.match(stripVTControlCharacters(result.stderr), /status: 37\b/, JSON.stringify(colour));
      assert.deepEqual(readdirSync(scratch), []);
    } finally {
      rmSync(scratch, { recursive: true, force: true });
    }
  }
});

test("graph and telemetry share Mac classification for supported and custom runner labels", () => {
  for (const [runner, expectedMacos] of [
    [["macos-latest"], true],
    [["ubuntu-latest", "custom-macos-label"], false],
    [["windows-latest"], false],
    [["self-hosted", "macos-15"], true],
  ]) {
    const rows = jobs(
      { jobs: { proof: { "runs-on": runner } } },
      { event: "pull_request", action: "opened" },
      { macosRequired: true },
    );
    assert.equal(rows.length, 1);
    assert.equal(rows[0].macos, expectedMacos);
    const current = recording();
    current.scenarios[0].runs = [
      {
        workflow: "labels.yml",
        event: "pull_request",
        rows,
        macJobs: rows.filter((row) => row.macos).length,
      },
    ];
    current.lifecycles[0].runs = structuredClone(current.scenarios[0].runs);
    current.lifecycles[0].macRuns = current.lifecycles[0].runs.filter(
      (run) => run.macJobs > 0,
    ).length;
    assert.doesNotThrow(() => validateTelemetry(current));
    current.scenarios[0].runs[0].rows[0].macos = !expectedMacos;
    assert.throws(() => validateTelemetry(current), /Mac|macos|runner/);
  }
});
