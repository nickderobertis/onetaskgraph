// A scratch copy of this repository's real project graph whose coverage-unit command is a stub,
// so a test can drive the real Nx, the real merge and the real floor without an instrumented
// build. Nx's cache keys, the unit list and `workspace:coverage` are the repository's own; only
// what a unit executes is replaced. coverage-spawned-binary.test.mjs drives the real command.
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import {
  chmodSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { fixtureEnvironment } from "../../../fixture-environment.mjs";

export const root = fileURLToPath(new URL("../../../../", import.meta.url));
const worker = join(root, ".github/nx-policy/tests/fixtures/nx-workspace-contracts.mjs");
export const onLinux = process.platform === "linux";

export function scratchRoot(name) {
  return mkdtempSync(join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir(), `${name}-`));
}

export const environment = (additions = {}) => ({
  ...fixtureEnvironment(),
  NODE_TEST_CONTEXT: undefined,
  GITHUB_ACTIONS: undefined,
  CI: undefined,
  NX_DAEMON: "false",
  NX_NO_CLOUD: "true",
  NX_ISOLATE_PLUGINS: "false",
  FORCE_COLOR: "0",
  NX_TUI: "false",
  GIT_DIR: undefined,
  GIT_WORK_TREE: undefined,
  GIT_INDEX_FILE: undefined,
  GIT_COMMON_DIR: undefined,
  ONETASKGRAPH_TESTS_UNDER_COVERAGE: "1",
  ...additions,
});

export function git(cwd, ...args) {
  return execFileSync("git", args, { cwd, env: environment(), encoding: "utf8" });
}

// A copy of the tree as it stands, tracked changes included.
export function copyGraph(scratch, name) {
  const dir = join(scratch, name);
  git(root, "clone", "--quiet", "--shared", "--no-checkout", root, dir);
  git(dir, "checkout", "--quiet", "--detach", "HEAD");
  const diff = git(root, "diff", "--binary", "HEAD");
  if (diff) execFileSync("git", ["apply", "-"], { cwd: dir, env: environment(), input: diff });
  symlinkSync(join(root, "node_modules"), join(dir, "node_modules"), "dir");
  symlinkSync(join(root, ".github/node_modules"), join(dir, ".github/node_modules"), "dir");
  writeFileSync(join(dir, ".git/info/exclude"), "/node_modules\n/.github/node_modules\n/.stub\n");
  git(dir, "add", "-A");
  return dir;
}

export const projectFile = (crate) => `crates/${crate}/project.json`;
export function readJson(dir, path) {
  const value = JSON.parse(readFileSync(join(dir, path), "utf8"));
  assert.ok(
    value !== null && typeof value === "object" && !Array.isArray(value),
    `${path} is an object`,
  );
  return value;
}
export function writeJson(dir, path, value) {
  writeFileSync(join(dir, path), `${JSON.stringify(value, null, 2)}\n`);
}

export function e2eUnits(dir) {
  return readdirSync(join(dir, "crates"))
    .filter((crate) => {
      try {
        return readJson(dir, projectFile(crate)).tags?.includes("layer:e2e");
      } catch {
        return false;
      }
    })
    .sort();
}

export function coverageUnits(dir) {
  const entry = readJson(dir, "workspace/project.json").targets.coverage.dependsOn.find(
    (candidate) => candidate.target === "coverage",
  );
  assert.ok(entry, "workspace:coverage lists its units");
  return [...entry.projects].sort();
}

const STUB_UNIT = `#!/usr/bin/env bash
set -euo pipefail
if [ "\${1:-}" = --tool-version ]; then exec bash "$(dirname "$0")/../.stub/real-unit.sh" "$@"; fi
unit="$1"
mkdir -p target/coverage-units
echo "$unit" >> .stub/ran
if [ -f .stub/no-report ]; then exit 0; fi
if [ -f ".stub/lcov/$unit" ]; then
  cp ".stub/lcov/$unit" "target/coverage-units/$unit.lcov"
else
  n="$(printf '%s' "$unit" | cksum | cut -d' ' -f1)"
  {
    printf 'SF:crates/%s/src/lib.rs\\nDA:1,1\\nDA:2,1\\nDA:3,%s\\nend_of_record\\n' "$unit" "$(( n % 2 ))"
    printf 'SF:crates/shared/src/lib.rs\\nDA:%s,1\\nDA:7,0\\nend_of_record\\n' "$(( n % 5 + 10 ))"
    # Volume, every line covered, so the default total clears the floor.
    printf 'SF:crates/%s/src/volume.rs\\n' "$unit"
    seq 1 400 | sed 's/.*/DA:&,1/'
    printf 'end_of_record\\n'
  } > "target/coverage-units/$unit.lcov"
fi
case "$unit" in
  onetaskgraph-linear-e2e | onetaskgraph-github-projects-e2e | onetaskgraph-live-janitor)
    mkdir -p "target/debug/telemetry/$unit"
    echo '{}' > "target/debug/telemetry/$unit/stub.json"
    ;;
esac
`;
const STUB_CARGO = `#!/usr/bin/env bash
# Stands in for the instrumented build only: every other invocation is the real cargo.
if [ "$1 $2" = "llvm-cov --no-report" ]; then exit 0; fi
for tool in llvm-cov nextest; do
  if [ "$1 $2" = "$tool --version" ] && [ -f "$(dirname "$0")/../wrap-$tool" ]; then
    echo "$tool from a wrapper"; exit 0
  fi
done
exec "$REAL_CARGO" "$@"
`;

export function stubbedGraph(scratch, name) {
  const dir = copyGraph(scratch, name);
  const stub = join(dir, ".stub");
  mkdirSync(join(stub, "bin"), { recursive: true });
  mkdirSync(join(stub, "lcov"));
  renameSync(join(dir, "scripts/coverage-unit.sh"), join(stub, "real-unit.sh"));
  writeFileSync(join(dir, "scripts/coverage-unit.sh"), STUB_UNIT);
  chmodSync(join(dir, "scripts/coverage-unit.sh"), 0o755);
  writeFileSync(join(stub, "bin/cargo"), STUB_CARGO);
  chmodSync(join(stub, "bin/cargo"), 0o755);
  writeFileSync(join(stub, "ran"), "");
  return dir;
}

const realCargo = () =>
  execFileSync("bash", ["-c", "command -v cargo"], { encoding: "utf8" }).trim();

export function nxEnvironment(dir, additions = {}) {
  const path = `${join(dir, ".stub/bin")}:${process.env.PATH}`;
  return environment({ PATH: path, REAL_CARGO: realCargo(), ...additions });
}

// One real Nx run of a target and what Nx's own output says happened to each task.
export function nxRun(dir, task = "workspace:coverage", additions = {}, flags = []) {
  const result = spawnSync(
    join(dir, "node_modules/.bin/nx"),
    ["run", task, "--output-style=stream", ...flags],
    {
      cwd: dir,
      env: nxEnvironment(dir, additions),
      encoding: "utf8",
      maxBuffer: 256 * 1024 * 1024,
    },
  );
  const output = `${result.stdout}\n${result.stderr}`.replace(ANSI, "");
  const executed = new Set();
  const replayed = new Set();
  for (const [, id, tag] of output.matchAll(/^> nx run (\S+?)(?:\s+\[([^\]]+)\])?\s*$/gm))
    (tag ? replayed : executed).add(id);
  return { status: result.status, output, executed, replayed };
}

const ANSI = new RegExp(`${String.fromCharCode(27)}\\[[0-9;]*m`, "g");
// `cargo metadata`'s document, held to the fields the key tests read.
export function cargoMetadata() {
  const value = JSON.parse(
    execFileSync("cargo", ["metadata", "--format-version", "1", "--all-features", "--locked"], {
      cwd: root,
      env: environment(),
      encoding: "utf8",
      maxBuffer: 128 * 1024 * 1024,
    }),
  );
  assert.ok(
    Array.isArray(value?.packages) &&
      value.packages.every((pkg) => typeof pkg?.id === "string" && typeof pkg?.name === "string") &&
      Array.isArray(value.workspace_members) &&
      Array.isArray(value.resolve?.nodes) &&
      value.resolve.nodes.every((node) => typeof node?.id === "string" && Array.isArray(node.deps)),
    "cargo metadata has packages, workspace_members and resolve.nodes",
  );
  return value;
}

export function recordedReadSet() {
  const recorded = JSON.parse(
    readFileSync(join(root, ".github/nx-policy/tests/fixtures/coverage-read-set.json"), "utf8"),
  );
  assert.ok(
    recorded !== null &&
      recorded.units !== null &&
      typeof recorded.units === "object" &&
      !Array.isArray(recorded.units) &&
      Object.values(recorded.units).every(
        (files) => Array.isArray(files) && files.every((file) => typeof file === "string"),
      ),
    "the recording maps each unit to the files it read",
  );
  return recorded;
}

export const unitTask = (unit) => `${unit}:coverage`;

export function ranUnits(run, units) {
  return units.filter((unit) => run.executed.has(unitTask(unit))).sort();
}
export function replayedUnits(run, units) {
  return units.filter((unit) => run.replayed.has(unitTask(unit))).sort();
}

export function assertSeeded(run, units, label) {
  assert.equal(run.status, 0, `${label}: ${run.output.slice(-3000)}`);
  assert.deepEqual(
    [...units].filter(
      (unit) => !run.executed.has(unitTask(unit)) && !run.replayed.has(unitTask(unit)),
    ),
    [],
    `${label}: every unit reported`,
  );
}

export function hashesOf(dir, ids, additions = {}) {
  const list = join(dirname(dir), `${Math.random().toString(36).slice(2)}.json`);
  writeFileSync(list, JSON.stringify(ids));
  try {
    const hashes = JSON.parse(
      execFileSync(process.execPath, [worker, list], {
        cwd: dir,
        env: nxEnvironment(dir, additions),
        encoding: "utf8",
        maxBuffer: 256 * 1024 * 1024,
      }),
    );
    assert.ok(
      ids.every(
        (id) =>
          typeof hashes[id]?.hash === "string" &&
          Array.isArray(hashes[id]?.inputs?.files) &&
          typeof hashes[id]?.nodes === "object",
      ),
      "Nx answered each task's hash and inputs",
    );
    return hashes;
  } finally {
    rmSync(list, { force: true });
  }
}

// LCOV by this test's own reader: file -> line -> covered.
export function readLcov(texts) {
  const files = new Map();
  for (const text of texts) {
    let file;
    for (const line of text.split("\n")) {
      if (line.startsWith("SF:")) {
        file = line.slice(3);
        if (!files.has(file)) files.set(file, new Map());
      } else if (line.startsWith("DA:")) {
        const [number, hits] = line.slice(3).split(",").map(Number);
        assert.ok(Number.isInteger(number) && Number.isInteger(hits), `malformed record ${line}`);
        const lines = files.get(file);
        lines.set(number, (lines.get(number) ?? false) || hits > 0);
      }
    }
  }
  return files;
}
export function totals(files) {
  let total = 0;
  let covered = 0;
  for (const lines of files.values())
    for (const hit of lines.values()) {
      total += 1;
      if (hit) covered += 1;
    }
  return { covered, total };
}
export function uncoveredByFile(files) {
  const out = {};
  for (const [file, lines] of files) {
    const missing = [...lines]
      .filter(([, hit]) => !hit)
      .map(([number]) => number)
      .sort((a, b) => a - b);
    if (missing.length) out[file] = missing;
  }
  return out;
}

export function reportedTotals(output) {
  const match = output.match(/lines covered: (\d+)[\s\S]*?lines total: (\d+)/);
  assert.ok(match, `the merge printed its totals: ${output.slice(-2000)}`);
  return { covered: Number(match[1]), total: Number(match[2]) };
}
