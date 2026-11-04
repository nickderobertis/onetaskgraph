import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../../", import.meta.url));
const env = {
  ...process.env,
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
