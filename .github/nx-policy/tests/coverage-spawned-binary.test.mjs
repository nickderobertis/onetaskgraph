import "../../test-setup.mjs";
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import { join } from "node:path";
import { after, test } from "node:test";
import {
  copyGraph,
  environment,
  nxRun,
  onLinux,
  readLcov,
  reportedTotals,
  root,
  scratchRoot,
  totals,
  unitTask,
} from "./fixtures/coverage-harness.mjs";

// An e2e unit runs the instrumented binary whether or not the binary's own coverage replayed:
// onetaskgraph:coverage-build rebuilds it, and the lines the journeys execute reach the merge.
// Real cargo-llvm-cov and real journeys, in a scratch copy of the graph.

const scratch = onLinux ? scratchRoot("coverage-spawned-binary") : "";
after(() => {
  if (onLinux) rmSync(scratch, { recursive: true, force: true });
});
const BINARY_FILE = "crates/onetaskgraph/src/main.rs";
const UNIT = "onetaskgraph-status-options-e2e";

test("an e2e unit finds and counts the instrumented binary after its coverage replayed", {
  skip: !onLinux,
  timeout: 3_000_000,
}, () => {
  const dir = copyGraph(scratch, "real");
  const warm = join(root, "target/llvm-cov-target");
  if (existsSync(warm)) {
    mkdirSync(join(dir, "target"), { recursive: true });
    execFileSync("cp", ["-al", warm, join(dir, "target/llvm-cov-target")]);
  }
  const seeded = nxRun(dir, unitTask("onetaskgraph"));
  assert.equal(seeded.status, 0, seeded.output.slice(-3000));
  const replay = nxRun(dir, unitTask("onetaskgraph"));
  assert.ok(replay.replayed.has(unitTask("onetaskgraph")), replay.output.slice(-2000));

  rmSync(join(dir, "target/llvm-cov-target/debug/onetaskgraph"), { force: true });
  const run = nxRun(dir, unitTask(UNIT));
  assert.equal(run.status, 0, run.output.slice(-3000));
  assert.ok(run.executed.has(unitTask(UNIT)), run.output.slice(-2000));
  assert.ok(run.executed.has("onetaskgraph:coverage-build"), "the binary was rebuilt");
  assert.ok(existsSync(join(dir, "target/llvm-cov-target/debug/onetaskgraph")));

  const report = readFileSync(join(dir, "target/coverage-units", `${UNIT}.lcov`), "utf8");
  const binaryLines = readLcov([report]).get(BINARY_FILE);
  assert.ok(binaryLines, `${UNIT}'s report names ${BINARY_FILE}`);
  assert.ok([...binaryLines.values()].some(Boolean), "the journeys executed its lines");

  const units = ["onetaskgraph", UNIT];
  const merge = spawnSync(
    "node",
    [
      "scripts/coverage-merge.mjs",
      "--fail-under-lines",
      "0",
      "--dir",
      "target/coverage-units",
      "--unit",
      ...units,
    ],
    { cwd: dir, env: environment(), encoding: "utf8" },
  );
  assert.equal(merge.status, 0, merge.stderr);
  const own = readLcov(
    units.map((unit) => readFileSync(join(dir, "target/coverage-units", `${unit}.lcov`), "utf8")),
  );
  assert.deepEqual(reportedTotals(merge.stdout), totals(own));
  for (const [number, hit] of binaryLines)
    if (hit) assert.equal(own.get(BINARY_FILE).get(number), true, `${BINARY_FILE}:${number}`);
});
