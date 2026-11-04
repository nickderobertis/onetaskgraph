import "../../test-setup.mjs";
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { fixtureEnvironment } from "../../fixture-environment.mjs";

const root = fileURLToPath(new URL("../../../", import.meta.url));
const env = {
  ...fixtureEnvironment(),
  GITHUB_ACTIONS: undefined,
  ONETASKGRAPH_SKIP_RUST_COVERAGE: "0",
  NX_DAEMON: "false",
  NX_ISOLATE_PLUGINS: "false",
  GIT_DIR: undefined,
  GIT_WORK_TREE: undefined,
  GIT_INDEX_FILE: undefined,
  GIT_COMMON_DIR: undefined,
};
const command = (program, args, additions = {}) =>
  execFileSync(program, args, {
    cwd: root,
    env: { ...env, ...additions },
    encoding: "utf8",
    maxBuffer: 128 * 1024 * 1024,
  });
const object = (value) => value !== null && typeof value === "object" && !Array.isArray(value);
const projects = JSON.parse(
  command(process.execPath, [".github/nx-policy/tests/fixtures/nx-workspace-contracts.mjs"]),
);
assert.ok(
  object(projects) &&
    Object.values(projects).every(
      (project) => object(project) && typeof project.root === "string" && object(project.targets),
    ),
);
const jobs = ["lint", "test-linux", "test-windows", "test-macos"];
const selected = Object.fromEntries(
  jobs.map((job) => {
    const output = command("bash", ["scripts/ci-job.sh", job, "--select-only"], {
      NX_BASE: "143002fe022eca174b58ccac06497800aedc9092",
      NX_HEAD: "HEAD",
    });
    const tasks = output
      .match(new RegExp(`ci-job: ${job}: selects ([^\\n]+)`))?.[1]
      .trim()
      .split(/\s+/);
    assert.ok(
      tasks?.length && tasks.every((id) => /^[\w@./-]+:[\w.-]+$/.test(id)),
      `${job}: expected Nx's selected task ids`,
    );
    return [job, tasks];
  }),
);
const allTasks = [...new Set(Object.values(selected).flat())];
function outputAllowed(output) {
  if (typeof output !== "string") return false;
  if (/\/telemetry\//.test(output)) return /\.(?:json|jsonl)$/.test(output);
  return /\/(?:[^/]*(?:coverage|report|telemetry|result|budget)[^/]*\.(?:json|jsonl|lcov)|\.coverage)$/.test(
    output,
  );
}
function problems(graph) {
  const failures = [];
  for (const id of allTasks) {
    const [project, name] = id.split(":");
    const target = graph[project]?.targets[name];
    assert.ok(object(target), `${id}: selected task must exist in Nx's effective graph`);
    if (target.cache === true && !target.inputs?.includes("{workspaceRoot}/.tool-versions"))
      failures.push(`${id}: cached tasks require the exact installed tool versions`);
    if (target.cache === true && !target.inputs?.includes("versionFreeNpm"))
      failures.push(`${id}: cached tasks require the locked Nx and JavaScript tools`);
    if (
      target.cache === true &&
      (!Array.isArray(target.outputs) || !target.outputs.every(outputAllowed))
    )
      failures.push(`${id}: outputs must name only profile, report or telemetry files`);
    const osLegs = jobs.filter((job) => job !== "lint" && selected[job].includes(id));
    if (
      osLegs.length > 1 &&
      (!Array.isArray(target.inputs) ||
        !target.inputs.some(
          (input) => object(input) && input.runtime === "node -p process.platform",
        ))
    )
      failures.push(`${id}: multiple OS legs require process.platform`);
  }
  return failures;
}

test("every selected task keeps its OS input and cached tasks store named files", () => {
  assert.ok(
    allTasks.length >= 147,
    "the real CI selection must include the original task inventory",
  );
  assert.deepEqual(problems(projects), []);
});

test("a directory, whole-directory glob or binary output anywhere and a lost platform input are refused", () => {
  for (const output of [
    "{workspaceRoot}/target/debug",
    "{workspaceRoot}/elsewhere/**/*",
    "{workspaceRoot}/somewhere/test-binary",
    "{workspaceRoot}/dist/**/*.json",
  ]) {
    const edited = structuredClone(projects);
    edited["onetaskgraph-core"].targets.test.outputs = [output];
    assert.deepEqual(problems(edited), [
      "onetaskgraph-core:test: outputs must name only profile, report or telemetry files",
    ]);
  }
  const edited = structuredClone(projects);
  edited["onetaskgraph-core"].targets.test.inputs = edited[
    "onetaskgraph-core"
  ].targets.test.inputs.filter(
    (input) => !object(input) || input.runtime !== "node -p process.platform",
  );
  assert.deepEqual(problems(edited), [
    "onetaskgraph-core:test: multiple OS legs require process.platform",
  ]);
});

test("an uncached target on multiple OS legs also requires its platform input", () => {
  const edited = structuredClone(projects);
  edited.onetaskgraph.targets.build.inputs = (
    edited.onetaskgraph.targets.build.inputs ?? []
  ).filter((input) => !object(input) || input.runtime !== "node -p process.platform");
  assert.deepEqual(problems(edited), [
    "onetaskgraph:build: multiple OS legs require process.platform",
  ]);
});

test("a cached target that loses the tool version file is refused", () => {
  const edited = structuredClone(projects);
  edited["onetaskgraph-core"].targets.test.inputs = edited[
    "onetaskgraph-core"
  ].targets.test.inputs.filter((input) => input !== "{workspaceRoot}/.tool-versions");
  assert.deepEqual(problems(edited), [
    "onetaskgraph-core:test: cached tasks require the exact installed tool versions",
  ]);
});

test("the CI pin reader emits exact file versions and refuses missing, ranged or duplicate pins", () => {
  const dir = mkdtempSync(join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir(), "tool-pins-"));
  try {
    mkdirSync(join(dir, "scripts"));
    for (const file of [
      ".tool-versions",
      "justfile",
      "scripts/scoped-release-plz.sh",
      "scripts/ci-tool-pins.sh",
    ])
      copyFileSync(join(root, file), join(dir, file));
    const output = join(dir, "outputs");
    const original = readFileSync(join(dir, ".tool-versions"), "utf8");
    const run = () => {
      writeFileSync(output, "");
      return spawnSync("bash", ["scripts/ci-tool-pins.sh"], {
        cwd: dir,
        env: { ...env, GITHUB_OUTPUT: output },
        encoding: "utf8",
      });
    };
    const result = run();
    assert.equal(result.status, 0, result.stdout + result.stderr);
    const pins = Object.fromEntries(
      readFileSync(output, "utf8")
        .trim()
        .split("\n")
        .map((line) => line.split("=")),
    );
    for (const line of original.split(/\r?\n/).filter((line) => line && !line.startsWith("#"))) {
      const [tool, version] = line.split(/\s+/);
      assert.equal(pins[tool], version, `${tool}: emitted tracked exact version`);
    }
    assert.equal(pins["cargo-nextest"], "0.9.150");
    for (const changed of [
      original.replace(/^cargo-nextest .*\n/m, ""),
      original.replace("cargo-nextest 0.9.150", "cargo-nextest >=0.9.150"),
      original.replace("cargo-nextest 0.9.150", "cargo-nextest 0.9.150 0.9.151"),
      `${original}cargo-nextest 0.9.150\n`,
    ]) {
      writeFileSync(join(dir, ".tool-versions"), changed);
      const refused = run();
      assert.notEqual(refused.status, 0);
      assert.match(refused.stderr, /no single pinned version for cargo-nextest/);
      assert.equal(readFileSync(output, "utf8"), "", "no partial install outputs");
    }
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
