import "../../test-setup.mjs";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync, rmSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { after, test } from "node:test";
import {
  coverageUnits,
  copyGraph,
  e2eUnits,
  environment,
  hashesOf,
  onLinux,
  root,
  scratchRoot,
  unitTask,
} from "./fixtures/coverage-harness.mjs";

// Holds every coverage unit's key to every workspace file its build or journeys can read. Two
// sources, because neither sees everything: this scan finds what a unit's Rust names (includes,
// `#[path]`, literal workspace paths, on every path a journey can take), and
// fixtures/coverage-read-set.json records what a real traced run opened (strace of each unit) and
// compiled (cargo's dep-info), which a scan cannot see in what a journey spawns.

const E2E = new Set(e2eUnits(root));

// Read through a runtime input rather than as a file: their version is blanked.
const RUNTIME_HASHED =
  /^(sdks\/python\/src\/onetaskgraph_sdk\/__init__\.py|sdks\/typescript\/src\/index\.ts|Cargo\.toml|Cargo\.lock|crates\/[^/]+\/Cargo\.toml|\.cargo\/config\.toml|rust-toolchain\.toml|\.tool-versions|package\.json|bun\.lock|pyproject\.toml|uv\.lock|sdks\/[^/]+\/package\.json|sdks\/[^/]+\/pyproject\.toml|sdks\/python\/uv\.lock)$/;
const ROOTS = /^(crates|docs|sdks|scripts|\.github|config|screenshots|workspace)\//;
const SKIP_DIR = new Set([
  "target",
  "node_modules",
  ".venv",
  ".git",
  ".nx",
  "__pycache__",
  ".mypy_cache",
]);

function walk(start, into = []) {
  if (!existsSync(start)) return into;
  if (statSync(start).isFile()) {
    into.push(start);
    return into;
  }
  for (const name of readdirSync(start)) {
    if (SKIP_DIR.has(name)) continue;
    walk(join(start, name), into);
  }
  return into;
}

const statOf = (literal) =>
  existsSync(join(root, literal)) ? statSync(join(root, literal)) : undefined;

function addTarget(path, found) {
  if (!existsSync(path)) return;
  const inside = relative(root, path);
  // A bare ancestor (`"../.."`) names where a journey starts from, not what it reads.
  if (
    inside === "" ||
    inside === "crates" ||
    inside.startsWith("..") ||
    /^target(\/|$)/.test(inside)
  )
    return;
  for (const file of walk(path)) found.add(relative(root, file));
}

// `dependency` is a crate another unit compiles: its `#[cfg(test)]` tail is compiled only for
// its own tests, so what that tail names is no other unit's read.
function scanRust(file, crate, found, dependency = false) {
  let text = readFileSync(file, "utf8");
  if (dependency) text = text.split(/^\s*#\[cfg\(test\)\]/m)[0];
  const dir = dirname(file);
  const crateDir = join(root, "crates", crate);
  // An include or `#[path]` is relative to its own file; any other `../` literal is joined onto
  // the crate's manifest directory.
  const fileRelative = new Set();
  for (const [, literal] of text.matchAll(/include(?:_str|_bytes)?!\(\s*"([^"]+)"/g)) {
    fileRelative.add(literal);
    addTarget(resolve(dir, literal), found);
  }
  for (const [, literal] of text.matchAll(/#\[path\s*=\s*"([^"]+)"\]/g)) {
    fileRelative.add(literal);
    addTarget(resolve(dir, literal), found);
  }
  for (const [, literal] of text.matchAll(/"((?:\.\.\/)+[\w@+.-][\w@+./-]*)"/g))
    if (!fileRelative.has(literal)) addTarget(resolve(crateDir, literal), found);
  // `"sdks/python"` is a working directory, not a read; a deeper directory is a data set.
  for (const [, literal] of text.matchAll(/"([\w.-]+\/[\w@+./-]*)"/g))
    if (ROOTS.test(literal) && (statOf(literal)?.isFile() || literal.split("/").length > 2))
      addTarget(join(root, literal), found);
}

const rustFilesOf = (crate, sub) =>
  walk(join(root, "crates", crate, sub)).filter((file) => file.endsWith(".rs"));

function compiledCrates(unit, metadata) {
  const members = new Set(
    metadata.packages.filter((pkg) => pkg.source === null).map((pkg) => pkg.name),
  );
  const names = new Map(metadata.packages.map((pkg) => [pkg.id, pkg.name]));
  const nodes = new Map(
    metadata.resolve.nodes.map((node) => [
      names.get(node.id),
      node.deps.map((dep) => ({
        name: names.get(dep.pkg),
        normal: dep.dep_kinds.some((kind) => kind.kind !== "dev"),
      })),
    ]),
  );
  const out = new Set();
  const frontier = [...(nodes.get(unit) ?? []).map((dep) => dep.name)];
  if (E2E.has(unit)) frontier.push("onetaskgraph");
  while (frontier.length) {
    const name = frontier.pop();
    if (out.has(name) || !members.has(name)) continue;
    out.add(name);
    for (const dep of nodes.get(name) ?? []) if (dep.normal) frontier.push(dep.name);
  }
  return out;
}

function scannedReads(unit, metadata) {
  const found = new Set();
  for (const sub of ["src", "tests", "benches", "build.rs"])
    for (const file of rustFilesOf(unit, sub)) scanRust(file, unit, found);
  for (const crate of compiledCrates(unit, metadata))
    for (const file of rustFilesOf(crate, "src")) scanRust(file, crate, found, true);
  return found;
}

function unhashed(reads, hashed) {
  return [...reads].filter((file) => !hashed.has(file) && !RUNTIME_HASHED.test(file)).sort();
}

const recorded = JSON.parse(
  readFileSync(join(root, ".github/nx-policy/tests/fixtures/coverage-read-set.json"), "utf8"),
);
assert.ok(
  recorded !== null &&
    typeof recorded.units === "object" &&
    Object.values(recorded.units).every(
      (files) => Array.isArray(files) && files.every((file) => typeof file === "string"),
    ),
  "the recording maps each unit to the files it read",
);
const unitReads = (unit, file) => (recorded.units[unit] ?? []).includes(file);

const scratch = onLinux ? scratchRoot("coverage-read-set") : "";
after(() => {
  if (onLinux) rmSync(scratch, { recursive: true, force: true });
});

test("every workspace file a unit reads is in its task hash", { skip: !onLinux }, () => {
  const metadata = JSON.parse(
    execFileSync("cargo", ["metadata", "--format-version", "1", "--all-features", "--locked"], {
      cwd: root,
      env: environment(),
      encoding: "utf8",
      maxBuffer: 128 * 1024 * 1024,
    }),
  );
  const dir = copyGraph(scratch, "read-set");
  const units = coverageUnits(dir);
  const hashes = hashesOf(dir, units.map(unitTask));
  const problems = [];
  const tracked = new Set(
    execFileSync("git", ["ls-files"], {
      cwd: root,
      env: environment(),
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
    })
      .trim()
      .split("\n"),
  );
  // RUNTIME_HASHED is exempt from the file list only because each unit's runtime inputs hash it.
  const runtimeInput = (file) =>
    /python|uv\.lock|__init__\.py/.test(file)
      ? "--ecosystem python"
      : /index\.ts$/.test(file)
        ? "--ecosystem files"
        : /Cargo\.(toml|lock)$/.test(file)
          ? "nx-cargo-closure"
          : "--ecosystem npm";
  for (const unit of units) {
    const nodes = Object.keys(hashes[unitTask(unit)].nodes).join("\n");
    for (const file of tracked)
      if (RUNTIME_HASHED.test(file) && !nodes.includes(runtimeInput(file)) && unitReads(unit, file))
        problems.push(`${unit}: reads ${file}, which no runtime input of it hashes`);
  }
  for (const unit of units) {
    const hashed = new Set(hashes[unitTask(unit)].inputs.files);
    const reads = new Set(
      [...scannedReads(unit, metadata), ...(recorded.units[unit] ?? [])].filter((file) =>
        tracked.has(file),
      ),
    );
    for (const file of unhashed(reads, hashed)) problems.push(`${unit}: reads ${file}`);
  }
  assert.deepEqual(problems, []);
});

test("a file only a journey reads, left out of the inputs, is named", { skip: !onLinux }, () => {
  // docs/metadata.md is read only at runtime, by a journey joining it onto CARGO_MANIFEST_DIR.
  const hashed = new Set(["crates/onetaskgraph-core/src/lib.rs"]);
  assert.deepEqual(unhashed(new Set(["docs/metadata.md"]), hashed), ["docs/metadata.md"]);
});

test("a journey's literal path is found on the path that names it", { skip: !onLinux }, () => {
  const metadata = JSON.parse(
    execFileSync("cargo", ["metadata", "--format-version", "1", "--all-features", "--locked"], {
      cwd: root,
      env: environment(),
      encoding: "utf8",
      maxBuffer: 128 * 1024 * 1024,
    }),
  );
  assert.ok(scannedReads("onetaskgraph-core", metadata).has("docs/metadata.md"));
  assert.ok(scannedReads("onetaskgraph-e2e", metadata).has("README.md"));
});
