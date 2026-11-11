import "../../test-setup.mjs";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  linkSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { join } from "node:path";
import { after, test } from "node:test";
import {
  copyGraph,
  environment,
  nxRun,
  onLinux,
  readJson,
  root,
  scratchRoot,
  writeJson,
} from "./fixtures/coverage-harness.mjs";

// `onetaskgraph-e2e:budgets` reads the telemetry its `test` recorded, and on macOS and Windows
// that `test` is often replayed from the cache rather than run. This drives the real Nx, the
// real `budgets` target and onebudgetspec over a scratch copy of the graph, with the budget's
// real reader — this tree's `budget_runner::report_boundary` — after `test` was replayed: only
// what `test` executes and the build it waits on are replaced, so the replay restoring the
// recording, or not, is the repository's own declaration.

// Inside this checkout's target directory, so the runner's executable can be hard-linked.
const scratch = onLinux ? scratchRoot("budget-telemetry-replay", join(root, "target")) : "";
after(() => {
  if (onLinux) rmSync(scratch, { recursive: true, force: true });
});
const BUDGET = "boundary-invocations-per-public-copy";
const RECORDING = `target/debug/telemetry/onetaskgraph-e2e/${BUDGET}.json`;
const RESULTS = "target/budget-results/onetaskgraph-e2e-budget-results.json";
const RUNNER = "target/debug/deps/e2e-replayed";

// The workload the journey records under, read from the constant the reader holds it to.
function workload() {
  const source = readFileSync(
    join(root, "crates/onetaskgraph-e2e/tests/e2e/boundary/batch.rs"),
    "utf8",
  );
  const literal = source.match(/const BOUNDARY_WORKLOAD: &str = "((?:[^"\\]|\\.)*)";/s)?.[1];
  assert.ok(literal, "batch.rs declares BOUNDARY_WORKLOAD");
  // A Rust string continuation: a backslash, the line break and the next line's indentation.
  return literal.replace(/\\\n\s*/g, "");
}

// This tree's e2e test executable, built plainly, which `cargo test --test e2e` would run.
function runner() {
  const messages = execFileSync(
    "cargo",
    [
      "test",
      "--quiet",
      "-p",
      "onetaskgraph-e2e",
      "--all-features",
      "--locked",
      "--test",
      "e2e",
      "--no-run",
      "--message-format=json",
    ],
    { cwd: root, env: environment(), encoding: "utf8", maxBuffer: 256 * 1024 * 1024 },
  );
  const executable = messages
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line))
    .find(
      (message) => message.reason === "compiler-artifact" && message.target?.name === "e2e",
    )?.executable;
  assert.ok(executable, "cargo built the e2e test executable");
  return executable;
}

// `cargo test -p onetaskgraph-e2e … --test e2e -- <arguments>` runs the e2e executable from the
// package's root with <arguments>; here it runs the one built from this tree. Every other
// invocation is the real cargo.
const STUB_CARGO = `#!/usr/bin/env bash
set -euo pipefail
copy="$(cd "$(dirname "$0")/../.." && pwd)"
words=" $* "
if [ "$1" = test ] && [[ "$words" == *" -p onetaskgraph-e2e "* ]] && [[ "$words" == *" --test e2e "* ]]; then
  while [ "$#" -gt 0 ] && [ "$1" != -- ]; do shift; done
  [ "$#" -gt 0 ] && shift
  cd "$copy/crates/onetaskgraph-e2e"
  exec "$copy/${RUNNER}" "$@"
fi
exec "$REAL_CARGO" "$@"
`;

function replayGraph(name, executable, declareOutputs) {
  const dir = copyGraph(scratch, name);
  const stub = join(dir, ".stub");
  mkdirSync(join(stub, "bin"), { recursive: true });
  writeFileSync(join(stub, "bin/cargo"), STUB_CARGO);
  chmodSync(join(stub, "bin/cargo"), 0o755);
  mkdirSync(join(dir, "target/debug/deps"), { recursive: true });
  try {
    linkSync(executable, join(dir, RUNNER));
  } catch {
    copyFileSync(executable, join(dir, RUNNER));
  }
  writeFileSync(
    join(stub, "recording.json"),
    `${JSON.stringify({ budget: BUDGET, value: 3, detail: "recorded by the stand-in test", workload: workload() })}\n`,
  );
  writeFileSync(
    join(stub, "test.sh"),
    `set -euo pipefail\nmkdir -p "$(dirname ${RECORDING})"\ncp .stub/recording.json ${RECORDING}\necho test >> .stub/ran\n`,
  );
  const binary = readJson(dir, "crates/onetaskgraph/project.json");
  binary.targets.build.options.command = "true";
  writeJson(dir, "crates/onetaskgraph/project.json", binary);
  const e2e = readJson(dir, "crates/onetaskgraph-e2e/project.json");
  e2e.targets.test.options.command = "bash .stub/test.sh";
  if (!declareOutputs) e2e.targets.test.outputs = [];
  writeJson(dir, "crates/onetaskgraph-e2e/project.json", e2e);
  return dir;
}

// Run `test` once so the cache holds it, lose what it wrote, then ask for `budgets`.
function budgetsAfterReplay(dir) {
  const seeded = nxRun(dir, "onetaskgraph-e2e:test");
  assert.equal(seeded.status, 0, seeded.output.slice(-3000));
  assert.ok(seeded.executed.has("onetaskgraph-e2e:test"), seeded.output.slice(-2000));
  assert.ok(existsSync(join(dir, RECORDING)));
  rmSync(join(dir, "target/debug/telemetry"), { recursive: true });
  writeFileSync(join(dir, ".stub/ran"), "");
  const run = nxRun(dir, "onetaskgraph-e2e:budgets");
  assert.ok(run.replayed.has("onetaskgraph-e2e:test"), run.output.slice(-2000));
  assert.equal(readFileSync(join(dir, ".stub/ran"), "utf8"), "", "test was replayed, not run");
  assert.ok(run.executed.has("onetaskgraph-e2e:budgets"), run.output.slice(-2000));
  return run;
}

let executable;
const built = () => {
  executable ??= runner();
  return executable;
};

test("the budget reads its figure after its test was replayed from the cache, and passes", {
  skip: !onLinux,
  timeout: 3_000_000,
}, () => {
  const dir = replayGraph("declared", built(), true);
  const run = budgetsAfterReplay(dir);
  assert.equal(run.status, 0, run.output.slice(-3000));
  assert.ok(existsSync(join(dir, RECORDING)), "the replay restored the recording");
  assert.match(run.output, new RegExp(`budget ${BUDGET}: actual 3 processes, budget 3 processes`));
  assert.match(readFileSync(join(dir, RESULTS), "utf8"), new RegExp(BUDGET));
});

test("a test whose outputs leave out its telemetry leaves the replayed budget nothing to read", {
  skip: !onLinux,
  timeout: 3_000_000,
}, () => {
  const dir = replayGraph("undeclared", built(), false);
  const run = budgetsAfterReplay(dir);
  assert.notEqual(run.status, 0, run.output.slice(-3000));
  assert.match(run.output, new RegExp(`no telemetry at \\S*${BUDGET}\\.json`));
});
