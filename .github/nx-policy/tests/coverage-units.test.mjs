import "../../test-setup.mjs";
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import {
  appendFileSync,
  chmodSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { join } from "node:path";
import { after, before, describe, test } from "node:test";
import {
  coverageUnits,
  environment,
  git,
  hashesOf,
  nxRun,
  onLinux,
  projectFile,
  ranUnits,
  readJson,
  readLcov,
  replayedUnits,
  reportedTotals,
  root,
  scratchRoot,
  stubbedGraph,
  totals,
  uncoveredByFile,
  unitTask,
  writeJson,
} from "./fixtures/coverage-harness.mjs";

// The cache keys of the repository's real coverage units, driven through real Nx in a scratch
// copy of its graph. A key that misses something a run reads is how a stale result would pass
// the floor, so every assertion is on what Nx reports it ran.

const E2E = [
  "onetaskgraph-e2e",
  "onetaskgraph-github-projects-e2e",
  "onetaskgraph-linear-e2e",
  "onetaskgraph-status-options-e2e",
];
const BINARY_LINKS = [
  "onetaskgraph",
  "onetaskgraph-core",
  "onetaskgraph-github-projects",
  "onetaskgraph-in-memory",
  "onetaskgraph-linear",
  "onetaskgraph-local-md",
  "onetaskgraph-plugin-api",
  "onetaskgraph-status-options",
];
const STATUS_E2E_TEST = "crates/onetaskgraph-status-options-e2e/tests/e2e/main.rs";
// The non-Cargo files a release rewrites, from a real release commit.
const VERSIONED_FILES = [
  "bun.lock",
  "pyproject.toml",
  "uv.lock",
  "npm/cli/package.json",
  "npm/platforms/darwin-arm64/package.json",
  "npm/platforms/linux-arm64/package.json",
  "npm/platforms/linux-x64/package.json",
  "npm/platforms/win32-x64/package.json",
  "sdks/python/pyproject.toml",
  "sdks/python/src/onetaskgraph_sdk/__init__.py",
  "sdks/typescript/package.json",
  "sdks/typescript/src/index.ts",
];
const KEEPS_VERSION = ["onetaskgraph-e2e", "onetaskgraph-linear-e2e"];

const scratch = onLinux ? scratchRoot("coverage-units") : "";
after(() => {
  if (onLinux) rmSync(scratch, { recursive: true, force: true });
});

const metadata = onLinux
  ? JSON.parse(
      execFileSync("cargo", ["metadata", "--format-version", "1", "--all-features", "--locked"], {
        cwd: root,
        env: environment(),
        encoding: "utf8",
        maxBuffer: 128 * 1024 * 1024,
      }),
    )
  : undefined;

function resolveGraph() {
  const names = new Map(metadata.packages.map((pkg) => [pkg.id, pkg.name]));
  const nodes = new Map();
  for (const node of metadata.resolve.nodes)
    nodes.set(
      names.get(node.id),
      node.deps.map((dep) => ({
        name: names.get(dep.pkg),
        kinds: dep.dep_kinds.map((kind) => kind.kind ?? "normal"),
      })),
    );
  return nodes;
}

// What a unit's build compiles: its own dependencies of every kind, then what those need to build.
function compiledBy(unit) {
  const nodes = resolveGraph();
  const seen = new Set([unit]);
  const frontier = (nodes.get(unit) ?? []).map((dep) => dep.name);
  while (frontier.length) {
    const name = frontier.pop();
    if (seen.has(name)) continue;
    seen.add(name);
    for (const dep of nodes.get(name) ?? [])
      if (dep.kinds.some((kind) => kind !== "dev")) frontier.push(dep.name);
  }
  return seen;
}

function unitsReadingCrateSource(units, crate) {
  return units
    .filter(
      (unit) =>
        compiledBy(unit).has(crate) ||
        (E2E.includes(unit) &&
          (BINARY_LINKS.includes(crate) || compiledBy("onetaskgraph").has(crate))),
    )
    .sort();
}

function narrowestLockedPackage(units) {
  const members = new Set(metadata.workspace_members);
  const external = metadata.packages
    .filter((pkg) => !members.has(pkg.id) && pkg.source)
    .map((pkg) => pkg.name);
  const nodes = resolveGraph();
  const reach = (start) => {
    const seen = new Set();
    const frontier = [start];
    while (frontier.length) {
      const name = frontier.pop();
      if (seen.has(name)) continue;
      seen.add(name);
      for (const dep of nodes.get(name) ?? []) frontier.push(dep.name);
    }
    return seen;
  };
  const closure = (unit) => {
    const out = reach(unit);
    if (E2E.includes(unit)) for (const name of reach("onetaskgraph")) out.add(name);
    return out;
  };
  let best;
  for (const name of new Set(external)) {
    const readers = units.filter((unit) => closure(unit).has(name));
    if (readers.length && (!best || readers.length < best.readers.length))
      best = { name, readers: readers.sort() };
  }
  assert.ok(best, "some locked package reaches a unit");
  return best;
}

function bumpVersion(dir) {
  const manifest = join(dir, "Cargo.toml");
  const version = readFileSync(manifest, "utf8").match(
    /\[workspace\.package\][^[]*?version = "([^"]+)"/,
  )?.[1];
  assert.ok(version, "the workspace declares a version");
  const next = `${version}9`;
  const replaceVersion = (path) => {
    const text = readFileSync(join(dir, path), "utf8");
    writeFileSync(
      join(dir, path),
      text
        .split("\n")
        .map((line) =>
          /^version = "/.test(line) || /onetaskgraph/.test(line)
            ? line.replace(`"${version}"`, `"${next}"`).replace(`"=${version}"`, `"=${next}"`)
            : line,
        )
        .join("\n"),
    );
  };
  replaceVersion("Cargo.toml");
  replaceVersion("crates/onetaskgraph/Cargo.toml");
  for (const crate of readdirSync(join(dir, "crates"))) {
    const path = `crates/${crate}/Cargo.toml`;
    try {
      replaceVersion(path);
    } catch {
      // not a crate directory
    }
  }
  const lock = join(dir, "Cargo.lock");
  writeFileSync(
    lock,
    readFileSync(lock, "utf8").replace(
      /(name = "onetaskgraph[^"]*"\nversion = )"[^"]+"/g,
      `$1"${next}"`,
    ),
  );
  for (const path of VERSIONED_FILES) {
    const text = readFileSync(join(dir, path), "utf8");
    writeFileSync(join(dir, path), text.replaceAll(version, next));
  }
  return VERSIONED_FILES;
}

describe("coverage unit keys", { skip: !onLinux }, () => {
  let dir;
  let units;
  before(() => {
    dir = stubbedGraph(scratch, "keys");
    units = coverageUnits(dir);
    const first = nxRun(dir);
    assert.equal(first.status, 0, first.output.slice(-3000));
    assert.deepEqual(ranUnits(first, units), units, "every unit runs on an empty cache");
  });

  function editing(paths, edit) {
    edit();
    try {
      return nxRun(dir);
    } finally {
      git(dir, "checkout", "--quiet", "HEAD", "--", ...paths);
    }
  }
  const touch = (path) => () => appendFileSync(join(dir, path), "\n// edited by a test\n");

  test("a second run with nothing changed replays every unit", () => {
    const run = nxRun(dir);
    assert.deepEqual(replayedUnits(run, units), units);
    assert.deepEqual(ranUnits(run, units), []);
  });

  test("a file no unit reads replays every unit", () => {
    const run = editing(["docs/task-projection.md"], touch("docs/task-projection.md"));
    assert.deepEqual(ranUnits(run, units), []);
    assert.deepEqual(replayedUnits(run, units), units);
  });

  const expectExactly = (path, expected) => {
    const run = editing([path], touch(path));
    assert.equal(run.status, 0, run.output.slice(-2000));
    assert.deepEqual(ranUnits(run, units), [...expected].sort(), path);
    assert.deepEqual(
      replayedUnits(run, units),
      units.filter((unit) => !expected.includes(unit)).sort(),
      `${path}: the rest replay`,
    );
  };

  test("editing a library crate's source re-runs exactly the units that compile it", () => {
    expectExactly(
      "crates/onetaskgraph-local-md/src/lib.rs",
      unitsReadingCrateSource(units, "onetaskgraph-local-md"),
    );
  });

  test("editing a crate's own tests re-runs that crate's unit alone", () => {
    const tests = readdirSync(join(root, "crates/onetaskgraph-local-md/tests")).find((name) =>
      name.endsWith(".rs"),
    );
    expectExactly(`crates/onetaskgraph-local-md/tests/${tests}`, ["onetaskgraph-local-md"]);
  });

  test("editing a dependency's source re-runs every unit that compiles it", () => {
    expectExactly(
      "crates/onetaskgraph-plugin-api/src/lib.rs",
      unitsReadingCrateSource(units, "onetaskgraph-plugin-api"),
    );
  });

  test("editing the binary's source re-runs its unit and every e2e unit", () => {
    expectExactly(
      "crates/onetaskgraph/src/main.rs",
      unitsReadingCrateSource(units, "onetaskgraph"),
    );
  });

  test("a Cargo.lock entry re-runs exactly the units whose build reaches that package", () => {
    const { name, readers } = narrowestLockedPackage(units);
    const lock = join(dir, "Cargo.lock");
    const run = editing(["Cargo.lock"], () => {
      const text = readFileSync(lock, "utf8");
      const edited = text.replace(new RegExp(`(name = "${name}"\\nversion = ")`), "$1999.");
      assert.notEqual(edited, text, `Cargo.lock names ${name}`);
      writeFileSync(lock, edited);
    });
    // The two units that keep the version hash the whole lock through --keep-version.
    assert.deepEqual(
      ranUnits(run, units),
      [...new Set([...readers, ...KEEPS_VERSION])].sort(),
      name,
    );
  });

  test("the toolchain file re-runs every unit", () => {
    const path = "rust-toolchain.toml";
    const run = editing([path], () => appendFileSync(join(dir, path), "\n# edited by a test\n"));
    assert.deepEqual(ranUnits(run, units), units);
  });

  for (const tool of ["cargo-llvm-cov", "cargo-nextest"])
    test(`the ${tool} pin re-runs every unit`, () => {
      const path = ".tool-versions";
      const run = editing([path], () =>
        writeFileSync(
          join(dir, path),
          readFileSync(join(dir, path), "utf8").replace(
            new RegExp(`^${tool} .*$`, "m"),
            `${tool} 9.9.9`,
          ),
        ),
      );
      assert.deepEqual(ranUnits(run, units), units, tool);
    });

  test("a file only one unit's journeys read is hashed by it alone", () => {
    // Left out of the inputs, the task hash no longer covers it: this is the gap the read-set test
    // reports by name. With it in, editing it re-runs that unit and replays the rest.
    const reader = "onetaskgraph-github-projects-e2e";
    const file = readFileSync(
      join(root, ".github/nx-policy/tests/fixtures/coverage-read-set.json"),
      "utf8",
    );
    const read = JSON.parse(file).units[reader].find((path) =>
      path.startsWith("docs/measurements/"),
    );
    assert.ok(read, "the recording names a measurement file its journeys read");
    const negative = stubbedGraph(scratch, "journey-read");
    const project = readJson(negative, projectFile(reader));
    project.targets.coverage.inputs = project.targets.coverage.inputs.filter(
      (input) => input !== "{workspaceRoot}/docs/measurements/**/*",
    );
    writeJson(negative, projectFile(reader), project);
    const left = hashesOf(negative, [unitTask(reader)])[unitTask(reader)].inputs.files;
    assert.ok(!left.includes(read), `${reader}'s key without the input leaves ${read} unhashed`);
    assert.ok(hashesOf(dir, [unitTask(reader)])[unitTask(reader)].inputs.files.includes(read));
    expectExactly(read, [reader]);
  });

  test("a version-only bump replays every unit but those whose key keeps the version", () => {
    const manifests = [
      ...git(dir, "ls-files", "Cargo.toml", "Cargo.lock", "crates/*/Cargo.toml").trim().split("\n"),
      ...VERSIONED_FILES,
    ];
    const run = editing(manifests, () => bumpVersion(dir));
    assert.deepEqual(ranUnits(run, units), [...KEEPS_VERSION].sort());
  });
});

// Each non-file input alone: every unit whose key names it executes, then replays once restored.
function nonFileInputs(dir) {
  const bin = join(dir, ".stub/bin");
  const executable = (name, body) => {
    writeFileSync(join(bin, name), body);
    chmodSync(join(bin, name), 0o755);
  };
  const real = (name) =>
    execFileSync("bash", ["-c", `command -v ${name}`], { encoding: "utf8" }).trim();
  const tools = execFileSync(
    "bash",
    ["-c", "rustc --print sysroot; rustc -vV | sed -n 's/^host: //p'"],
    {
      cwd: root,
      encoding: "utf8",
    },
  )
    .trim()
    .split("\n");
  const llvm = (name) => join(tools[0], "lib/rustlib", tools[1], "bin", name);
  const wrapper = (name, match, path) => ({
    set: () =>
      executable(
        name,
        `#!/usr/bin/env bash\nif [ "$*" = "${match}" ]; then echo "${name} from a wrapper"; exit 0; fi\nexec "${path}" "$@"\n`,
      ),
    unset: () => rmSync(join(bin, name), { force: true }),
  });
  const flag = (name) => ({
    set: () => writeFileSync(join(dir, ".stub", `wrap-${name}`), ""),
    unset: () => rmSync(join(dir, ".stub", `wrap-${name}`), { force: true }),
  });
  return {
    ONETASKGRAPH_TESTS_UNDER_COVERAGE: {
      env: { ONETASKGRAPH_TESTS_UNDER_COVERAGE: "" },
      set: () => {},
      unset: () => {},
    },
    rustc: wrapper("rustc", "-vV", real("rustc")),
    "cargo-llvm-cov": flag("llvm-cov"),
    "cargo-nextest": flag("nextest"),
    "llvm-profdata": wrapper("llvm-profdata", "--version", llvm("llvm-profdata")),
    "llvm-cov": wrapper("llvm-cov", "--version", llvm("llvm-cov")),
    "llvm-readobj": wrapper("llvm-readobj", "--version", llvm("llvm-readobj")),
    platform: {
      set: () =>
        executable(
          "node",
          `#!/usr/bin/env bash\nif [ "$*" = "-p process.platform" ]; then echo darwin; exit 0; fi\nexec "${process.execPath}" "$@"\n`,
        ),
      unset: () => rmSync(join(bin, "node"), { force: true }),
    },
  };
}

function nonFileInputFailures(dir, units, name, input) {
  const failures = [];
  input.set();
  const changed = nxRun(dir, "workspace:coverage", input.env ?? {});
  input.unset();
  for (const unit of units)
    if (!changed.executed.has(unitTask(unit)))
      failures.push(`${unit} replayed after ${name} changed`);
  const restored = nxRun(dir);
  for (const unit of units)
    if (!restored.replayed.has(unitTask(unit)))
      failures.push(`${unit} did not replay with ${name} restored`);
  return failures;
}

describe("each non-file input moves every key that names it", { skip: !onLinux }, () => {
  let dir;
  let units;
  let inputs;
  before(() => {
    dir = stubbedGraph(scratch, "non-file");
    units = coverageUnits(dir);
    inputs = nonFileInputs(dir);
    assert.equal(nxRun(dir).status, 0);
    assert.deepEqual(replayedUnits(nxRun(dir), units), units, "every unit replays to begin with");
  });

  for (const name of [
    "ONETASKGRAPH_TESTS_UNDER_COVERAGE",
    "rustc",
    "cargo-llvm-cov",
    "cargo-nextest",
    "llvm-profdata",
    "llvm-cov",
    "llvm-readobj",
    "platform",
  ])
    test(name, () => {
      assert.deepEqual(nonFileInputFailures(dir, units, name, inputs[name]), []);
    });

  test("the package version moves the two units that keep it, alone", () => {
    const manifests = [
      ...git(dir, "ls-files", "Cargo.toml", "Cargo.lock", "crates/*/Cargo.toml").trim().split("\n"),
      ...VERSIONED_FILES,
    ];
    bumpVersion(dir);
    const changed = nxRun(dir);
    git(dir, "checkout", "--quiet", "HEAD", "--", ...manifests);
    assert.deepEqual(ranUnits(changed, units), [...KEEPS_VERSION].sort());
    assert.deepEqual(replayedUnits(nxRun(dir), units), units);
  });

  test("an input taken out of a key is what the check names", () => {
    const negative = stubbedGraph(scratch, "non-file-negative");
    const allUnits = coverageUnits(negative);
    const nx = readJson(negative, "nx.json");
    nx.namedInputs.coverageUnit = nx.namedInputs.coverageUnit.filter(
      (input) => input.env !== "ONETASKGRAPH_TESTS_UNDER_COVERAGE",
    );
    writeJson(negative, "nx.json", nx);
    git(negative, "add", "nx.json");
    assert.equal(nxRun(negative).status, 0);
    const failures = nonFileInputFailures(negative, allUnits, "ONETASKGRAPH_TESTS_UNDER_COVERAGE", {
      env: { ONETASKGRAPH_TESTS_UNDER_COVERAGE: "" },
      set: () => {},
      unset: () => {},
    });
    assert.ok(
      failures.includes(`${allUnits[0]} replayed after ONETASKGRAPH_TESTS_UNDER_COVERAGE changed`),
      failures.join("\n") || "the taken-out input went unnoticed",
    );
  });
});

const lcovOf = (dir, units) =>
  units.map((unit) => readFileSync(join(dir, "target/coverage-units", `${unit}.lcov`), "utf8"));

function assertMergeMatchesFiles(dir, run, units) {
  const files = readLcov(lcovOf(dir, units));
  assert.deepEqual(reportedTotals(run.output), totals(files));
  // The merge names each file's uncovered lines below its floor: 100 makes it print them all.
  const every = spawnSync(
    "node",
    [
      "scripts/coverage-merge.mjs",
      "--fail-under-lines",
      "100",
      "--dir",
      "target/coverage-units",
      "--unit",
      ...units,
    ],
    { cwd: dir, env: environment(), encoding: "utf8" },
  );
  const reported = {};
  for (const [, file, lines] of every.stdout.matchAll(/^(crates\/\S+): ([\d,]+)$/gm))
    reported[file] = lines.split(",").map(Number);
  assert.deepEqual(reported, uncoveredByFile(files), "uncovered lines per file: zero differ");
}

describe("the real workspace:coverage over cached reports", { skip: !onLinux }, () => {
  const STALE_LCOV = "SF:crates/stale/src/lib.rs\nDA:1,0\nDA:2,0\nDA:3,0\nend_of_record\n";

  test("a merge with one unit executed and the rest replayed is exact", () => {
    const dir = stubbedGraph(scratch, "merge");
    const units = coverageUnits(dir);
    const seed = nxRun(dir);
    assert.equal(seed.status, 0, seed.output.slice(-3000));
    appendFileSync(join(dir, STATUS_E2E_TEST), "\n// edited by a test\n");
    const run = nxRun(dir);
    assert.equal(run.status, 0, run.output.slice(-3000));
    assert.deepEqual(ranUnits(run, units), ["onetaskgraph-status-options-e2e"]);
    assert.equal(replayedUnits(run, units).length, units.length - 1);
    assertMergeMatchesFiles(dir, run, units);
  });

  test("a report planted from another commit never reaches the total", () => {
    const dir = stubbedGraph(scratch, "stale");
    const units = coverageUnits(dir);
    assert.equal(nxRun(dir).status, 0);
    appendFileSync(join(dir, STATUS_E2E_TEST), "\n// edited by a test\n");
    const hashes = hashesOf(dir, units.map(unitTask));
    for (const unit of units)
      writeFileSync(join(dir, "target/coverage-units", `${unit}.lcov`), STALE_LCOV);
    const run = nxRun(dir);
    assert.equal(run.status, 0, run.output.slice(-3000));
    assert.deepEqual(ranUnits(run, units), ["onetaskgraph-status-options-e2e"]);
    for (const unit of units) {
      const report = readFileSync(join(dir, "target/coverage-units", `${unit}.lcov`), "utf8");
      assert.ok(!report.includes("crates/stale/"), `${unit}: nothing planted remains`);
      if (unit === "onetaskgraph-status-options-e2e") continue;
      const cached = readFileSync(
        join(
          dir,
          ".nx/cache",
          hashes[unitTask(unit)].hash,
          "target/coverage-units",
          `${unit}.lcov`,
        ),
        "utf8",
      );
      assert.equal(report, cached, `${unit}: restored byte for byte`);
    }
    assertMergeMatchesFiles(dir, run, units);
  });

  test("a file no unit owns refuses the merge, naming it", () => {
    const dir = stubbedGraph(scratch, "stray");
    assert.equal(nxRun(dir).status, 0);
    writeFileSync(join(dir, "target/coverage-units/left-behind.lcov"), STALE_LCOV);
    const run = nxRun(dir);
    assert.notEqual(run.status, 0);
    assert.match(run.output, /holds what no named unit owns: left-behind\.lcov/);
  });

  test("a named unit with no report refuses the merge, naming it", () => {
    const dir = stubbedGraph(scratch, "missing");
    writeFileSync(join(dir, ".stub/no-report"), "");
    const run = nxRun(dir, "workspace:coverage", {}, ["--skipNxCache"]);
    assert.notEqual(run.status, 0);
    assert.match(run.output, /no report for unit\(s\) .*onetaskgraph-core/);
  });

  // Enough covered lines in one file and few elsewhere to land the total where the case needs it.
  function lcovAt(dir, units, percent) {
    const total = 100000;
    const covered = Math.floor((total * percent) / 100);
    const body = Array.from(
      { length: total },
      (_, index) => `DA:${index + 1},${index < covered ? 1 : 0}`,
    ).join("\n");
    for (const unit of units)
      writeFileSync(
        join(dir, ".stub/lcov", unit),
        `SF:crates/one/src/lib.rs\n${body}\nend_of_record\n`,
      );
  }

  for (const percent of [92.5, 91])
    test(`a merged total of ${percent}% fails the floor, naming the uncovered lines`, () => {
      const dir = stubbedGraph(scratch, `floor-${percent}`);
      const units = coverageUnits(dir);
      lcovAt(dir, units, percent);
      const run = nxRun(dir, "workspace:coverage", {}, ["--skipNxCache"]);
      assert.equal(run.status, 1, run.output.slice(-3000));
      assert.match(run.output, /below the floor of 92\.88/);
      assert.match(run.output, /crates\/one\/src\/lib\.rs: \d+/);
    });

  test("a total at the floor passes", () => {
    const dir = stubbedGraph(scratch, "floor-pass");
    const units = coverageUnits(dir);
    lcovAt(dir, units, 92.88);
    const run = nxRun(dir, "workspace:coverage", {}, ["--skipNxCache"]);
    assert.equal(run.status, 0, run.output.slice(-3000));
  });

  test("a unit outside the merge's list is refused by the reconciliation", () => {
    const dir = stubbedGraph(scratch, "unlisted");
    const workspace = readJson(dir, "workspace/project.json");
    const entry = workspace.targets.coverage.dependsOn.find((item) => item.target === "coverage");
    entry.projects = entry.projects.filter((name) => name !== "onetaskgraph-local-md");
    writeJson(dir, "workspace/project.json", workspace);
    const run = nxRun(dir);
    assert.notEqual(run.status, 0);
    assert.match(run.output, /does not depend on 'onetaskgraph-local-md''s coverage target/);
  });

  test("every unit's output is its LCOV report under a name the merge reads", () => {
    const dir = stubbedGraph(scratch, "outputs");
    for (const unit of coverageUnits(dir)) {
      const target = readJson(dir, projectFile(unit)).targets.coverage;
      assert.equal(target.cache, true, unit);
      assert.ok(
        target.outputs.includes(`{workspaceRoot}/target/coverage-units/${unit}.lcov`),
        unit,
      );
    }
  });
});
