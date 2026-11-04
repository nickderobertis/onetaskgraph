import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, test } from "node:test";
import { fileURLToPath } from "node:url";
import { canonicalInputNames } from "../version-inputs.mjs";

const root = fileURLToPath(new URL("../../../", import.meta.url));
const worker = join(root, ".github/nx-policy/tests/fixtures/nx-workspace-contracts.mjs");
const baseline = "705faa9cc517af94977ddd10f82f5728d6d07725";
const prerelease = "f8a10df";
const firstCommit = "143002fe022eca174b58ccac06497800aedc9092";
const scratch = mkdtempSync(
  join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir(), "nx-hashes-"),
);
after(() => rmSync(scratch, { recursive: true, force: true }));
const environment = {
  ...process.env,
  NODE_TEST_CONTEXT: undefined,
  GITHUB_ACTIONS: undefined,
  ONETASKGRAPH_SKIP_RUST_COVERAGE: "0",
  NX_DAEMON: "false",
  NX_ISOLATE_PLUGINS: "false",
  GIT_DIR: undefined,
  GIT_WORK_TREE: undefined,
  GIT_INDEX_FILE: undefined,
  GIT_COMMON_DIR: undefined,
  GITHUB_RUN_ID: "1001",
  GITHUB_RUN_ATTEMPT: "1",
};
function command(cwd, program, args, input, additions = {}) {
  return execFileSync(program, args, {
    cwd,
    env: { ...environment, ...additions },
    input,
    encoding: "utf8",
    maxBuffer: 128 * 1024 * 1024,
  });
}
function fixture(name, ref, working = false) {
  const dir = join(scratch, name);
  command(root, "git", ["clone", "--quiet", "--shared", "--no-checkout", root, dir]);
  command(dir, "git", ["checkout", "--quiet", "--detach", ref]);
  if (working) {
    const diff = command(root, "git", ["diff", "--binary", "HEAD"]);
    if (diff) command(dir, "git", ["apply", "-"], diff);
  }
  mkdirSync(join(dir, ".github"), { recursive: true });
  symlinkSync(join(root, "node_modules"), join(dir, "node_modules"), "dir");
  symlinkSync(join(root, ".github/node_modules"), join(dir, ".github/node_modules"), "dir");
  writeFileSync(join(dir, ".git/info/exclude"), "/node_modules\n/.github/node_modules\n");
  command(dir, "git", ["add", "-A"]);
  return dir;
}
const jobs = ["lint", "test-linux", "test-windows", "test-macos"];
function selected(dir) {
  const tasks = jobs.flatMap((job) => {
    const output = execFileSync("bash", ["scripts/ci-job.sh", job, "--select-only"], {
      cwd: dir,
      env: { ...environment, NX_BASE: firstCommit, NX_HEAD: "HEAD" },
      encoding: "utf8",
      maxBuffer: 16 * 1024 * 1024,
    });
    const ids = output
      .match(new RegExp(`ci-job: ${job}: selects ([^\\n]+)`))?.[1]
      .trim()
      .split(/\s+/);
    assert.ok(
      ids?.length && ids.every((id) => /^[\w@./-]+:[\w.-]+$/.test(id)),
      `${job}: selected task ids`,
    );
    return ids;
  });
  return [...new Set(tasks)].sort();
}
const object = (value) => value !== null && typeof value === "object" && !Array.isArray(value);
function hashes(dir, ids, additions = {}) {
  rmSync(join(dir, ".nx"), { recursive: true, force: true });
  const list = join(scratch, "task-ids.json");
  writeFileSync(list, JSON.stringify(ids));
  const result = JSON.parse(command(dir, process.execPath, [worker, list], undefined, additions));
  assert.ok(
    object(result) &&
      ids.every(
        (id) =>
          object(result[id]) &&
          typeof result[id].hash === "string" &&
          Array.isArray(result[id].inputs?.files) &&
          result[id].inputs.files.every((file) => typeof file === "string") &&
          object(result[id].definition) &&
          object(result[id].nodes) &&
          Object.values(result[id].nodes).every((value) => typeof value === "string"),
      ),
    "Nx returns each task's hash, file details and effective definition",
  );
  return result;
}
const versionReaders = new Set([
  "onetaskgraph-e2e:test",
  "sdk-python:test",
  "sdk-python:coverage",
  "sdk-typescript:test",
  "sdk-typescript:coverage",
]);
const textReaders = new Set([
  "sdk-python:format-check",
  "sdk-python:lint",
  "sdk-typescript:format-check",
  "sdk-typescript:lint",
  "workspace:format-check",
  "workspace:lint",
  "workspace:typecheck",
]);

test("real Nx over the four jobs keeps the recorded v0.13.3 release hashes except named version and text readers", () => {
  const dir = fixture("release", "HEAD", true);
  // Keep current input definitions, but replay the recorded release's exact file bodies.
  // This also exercises the same comparison when HEAD is a later release version.
  const recordedFiles = command(root, "git", ["diff", "--name-only", prerelease, baseline])
    .trim()
    .split("\n");
  for (const file of recordedFiles)
    writeFileSync(join(dir, file), command(root, "git", ["show", `${baseline}:${file}`]));
  const ids = selected(dir);
  const released = hashes(dir, ids);
  const cached = ids.filter((id) => released[id].definition.cache === true);
  assert.ok(
    cached.length >= 109,
    "all original cached tasks except the approved uncached Rust coverage are included",
  );
  const patch = command(root, "git", ["diff", "--binary", prerelease, baseline]);
  assert.equal(
    command(root, "git", ["diff", "--name-only", prerelease, baseline]).trim().split("\n").length,
    27,
    "the recorded release changes exactly the repository's 27 release files",
  );
  command(dir, "git", ["apply", "--reverse", "-"], patch);
  const before = hashes(dir, cached);
  command(dir, "git", ["apply", "-"], patch);
  const after = hashes(dir, cached);
  const moved = cached.filter((id) => before[id].hash !== after[id].hash).sort();
  assert.deepEqual(
    moved,
    cached.filter((id) => versionReaders.has(id) || textReaders.has(id)).sort(),
    "release reruns are precisely the version-reading tests and raw text-reading checks",
  );
  for (const id of cached) {
    for (const runtime of ["cargo", "python"]) {
      const key = `runtime:node scripts/nx-version-free-inputs.mjs --ecosystem ${runtime}`;
      if (before[id].nodes[key] !== undefined)
        assert.equal(
          after[id].nodes[key],
          before[id].nodes[key],
          `${id}: ${runtime} digest is stable`,
        );
    }
  }
  console.log(
    `Recorded release: ${cached.length - moved.length}/${cached.length} cached hashes unchanged; rerun: ${moved.join(", ")}`,
  );
});

test("Nx's own hash file details retain every original cached target's file input", () => {
  const baseDir = fixture("base", baseline);
  const base = hashes(baseDir, selected(baseDir));
  assert.equal(
    Object.values(base).filter((task) => task.definition.cache === true).length,
    110,
    "the spike baseline contains all 110 cached tasks",
  );
  const currentDir = fixture("current", "HEAD", true);
  const current = hashes(currentDir, selected(currentDir));
  const tracked = command(currentDir, "git", ["ls-files", "-z"]).split("\0").filter(Boolean);
  const literals = existsSync(join(currentDir, ".github/nx-version-files"))
    ? readFileSync(join(currentDir, ".github/nx-version-files"), "utf8")
        .split(/\r?\n/)
        .filter((line) => line && !line.startsWith("#"))
    : [];
  const names = Object.fromEntries(
    Object.entries(canonicalInputNames(currentDir)).map(([ecosystem, names]) => [
      ecosystem,
      new Set(names),
    ]),
  );
  for (const [id, task] of Object.entries(base)) {
    assert.ok(current[id], `${id}: original task still runs`);
    const details = current[id];
    if (/^(onetaskgraph[^:]*|sdk-[^:]+):(test|test-uncovered|coverage)$/.test(id))
      assert.deepEqual(
        details.definition.options,
        task.definition.options,
        `${id}: no product test command was removed or split`,
      );
    if (task.definition.cache !== true) continue;
    assert.equal(
      details.definition.cache,
      id !== "onetaskgraph-linear:coverage",
      `${id}: only the approved Rust coverage target becomes uncached`,
    );
    const covered = new Set(details.inputs.files);
    for (const key of Object.keys(details.nodes)) {
      const runtime = key.match(
        /^runtime:node scripts\/nx-version-free-inputs\.mjs --ecosystem (cargo|npm|python|files)(?: --keep-version)?$/,
      )?.[1];
      if (!runtime) continue;
      for (const file of runtime === "files"
        ? literals
        : tracked.filter((file) => names[runtime].has(file.split("/").at(-1))))
        covered.add(file);
    }
    const lost = task.inputs.files.filter(
      (file) => !covered.has(file) && !file.endsWith("/CHANGELOG.md"),
    );
    assert.deepEqual(lost, [], `${id}: original files lost from raw or canonical Nx inputs`);
  }
});

function requireMoved(before, after, ids, rule) {
  for (const id of ids)
    assert.notEqual(after[id].hash, before[id].hash, `${id}: lost input ${rule}`);
}

test("every canonical ecosystem and named raw input invalidates real Nx hashes on substantive changes", () => {
  const dir = fixture("input-mutations", "HEAD", true);
  const initial = hashes(dir, selected(dir));
  const cached = Object.keys(initial).filter((id) => initial[id].definition.cache === true);
  const definition = JSON.parse(readFileSync(join(dir, "nx.json"), "utf8"));
  const cases = definition.namedInputs.sharedGlobals
    .filter((input) => typeof input === "string" && input.startsWith("{workspaceRoot}/"))
    .map((input) => {
      const file = input.slice("{workspaceRoot}/".length);
      return {
        file,
        rule: input,
        edit: file.endsWith(".json")
          ? (body) =>
              `${JSON.stringify(
                {
                  ...JSON.parse(body),
                  formatter: { ...JSON.parse(body).formatter, lineWidth: 99 },
                },
                null,
                2,
              )}\n`
          : file === ".python-version"
            ? (body) => body.replace("3.13", "3.14")
            : file === ".tool-versions"
              ? (body) => body.replace("just 1.51.0", "just 1.51.1")
              : file === "rust-toolchain.toml"
                ? (body) => body.replace('channel = "1.97.1"', 'channel = "1.97.0"')
                : (body) => `${body}\n# Nx input mutation\n`,
      };
    });
  assert.ok(definition.namedInputs.default.includes("{projectRoot}/**/*"));
  cases.push(
    {
      file: "crates/onetaskgraph-core/src/lib.rs",
      rule: "{projectRoot}/**/*",
      edit: (body) => `${body}\n// Nx source mutation\n`,
    },
    {
      file: "crates/onetaskgraph-core/tests/fixtures/stored-forms-wire.json",
      rule: "fixture",
      edit: (body) => `${body}\n`,
    },
    {
      file: "Cargo.toml",
      ecosystem: "cargo",
      rule: "dependency entry",
      edit: (body) => body.replace('async-trait = "0.1.92"', 'async-trait = "0.1.91"'),
    },
    {
      file: "crates/onetaskgraph-core/Cargo.toml",
      ecosystem: "cargo",
      rule: "feature",
      edit: (body) => body.replace("default = []", 'default = ["github-projects"]'),
    },
    {
      file: "Cargo.lock",
      ecosystem: "cargo",
      rule: "external dependency version",
      edit: (body) => body.replace('version = "0.8.12"', 'version = "0.8.13"'),
    },
    {
      file: "package.json",
      ecosystem: "npm",
      rule: "npm dependency",
      edit: (body) => {
        const value = JSON.parse(body);
        value.devDependencies["@onebudgetspec/cli"] = "0.1.4";
        return `${JSON.stringify(value, null, 2)}\n`;
      },
    },
    {
      file: "bun.lock",
      ecosystem: "npm",
      rule: "npm lock dependency",
      edit: (body) =>
        body.replace('"@onebudgetspec/cli": "0.1.5"', '"@onebudgetspec/cli": "0.1.4"'),
    },
    {
      file: "pyproject.toml",
      ecosystem: "python",
      rule: "Python build dependency",
      edit: (body) => body.replace("maturin>=1.9,<2", "maturin>=1.10,<2"),
    },
    {
      file: "sdks/python/pyproject.toml",
      ecosystem: "python",
      rule: "Python dependency",
      edit: (body) => body.replace("pydantic>=2.12", "pydantic>=2.13"),
    },
    {
      file: "uv.lock",
      ecosystem: "python",
      rule: "Python lock dependency",
      edit: (body) => body.replace('version = "0.8.0"', 'version = "0.8.1"'),
    },
  );
  for (const file of [
    ".github/cache-policy/project.json",
    ".github/nx-policy/project.json",
    "pool-maintenance/project.json",
  ])
    cases.push({
      file,
      rule: "workspace guard project inventory",
      edit: (body) => {
        const value = JSON.parse(body);
        value.targets.format.options.command += " ";
        return JSON.stringify(value, null, 2);
      },
    });
  cases.push(
    {
      file: "justfile",
      rule: "workspace guard command floor",
      edit: (body) => `${body}\n# Nx input mutation\n`,
    },
    {
      file: "crates/onetaskgraph-e2e/tests/e2e/document_store.py",
      rule: "product version discovery outside SDKs/scripts",
      edit: (body) => `${body}\n# Nx discovery mutation\n`,
    },
  );
  const literals = readFileSync(join(dir, ".github/nx-version-files"), "utf8")
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith("#"));
  for (const file of literals)
    cases.push({
      file,
      ecosystem: "files",
      rule: "non-version SDK source",
      edit: (body) =>
        body +
        (file.endsWith(".py")
          ? "\n# Nx literal-file mutation\n"
          : "\n// Nx literal-file mutation\n"),
    });
  const exercised = new Set();
  for (const { file, ecosystem, rule, edit } of cases) {
    const path = join(dir, file);
    const original = readFileSync(path, "utf8");
    const changed = edit(original);
    assert.notEqual(changed, original, `${rule}: mutation must change ${file}`);
    const key = ecosystem
      ? `runtime:node scripts/nx-version-free-inputs.mjs --ecosystem ${ecosystem}`
      : undefined;
    const readers = cached.filter(
      (id) =>
        initial[id].inputs.files.includes(file) || (key && initial[id].nodes[key] !== undefined),
    );
    assert.ok(readers.length > 0, `${rule}: ${file} has cacheable readers`);
    if (rule.startsWith("workspace guard") || rule.startsWith("product version discovery"))
      assert.ok(
        readers.includes("workspace:typecheck"),
        `${file}: workspace guard must read the file`,
      );
    writeFileSync(path, changed);
    try {
      const after = hashes(dir, readers);
      requireMoved(initial, after, readers, `${rule} (${file})`);
      if (key) {
        const semanticReaders = readers.filter((id) => initial[id].nodes[key] !== undefined);
        assert.ok(semanticReaders.length > 0, `${ecosystem}: runtime is exercised`);
        for (const id of semanticReaders)
          assert.notEqual(
            after[id].nodes[key],
            initial[id].nodes[key],
            `${id}: substantive ${ecosystem} digest changes`,
          );
        exercised.add(ecosystem);
      }
    } finally {
      writeFileSync(path, original);
    }
  }
  for (const inputs of Object.values(definition.namedInputs))
    for (const input of inputs) {
      const ecosystem = input?.runtime?.match(
        /^node scripts\/nx-version-free-inputs\.mjs --ecosystem ([a-z]+)$/,
      )?.[1];
      if (ecosystem)
        assert.ok(
          exercised.has(ecosystem),
          `${ecosystem}: every declared canonical rule was mutated`,
        );
    }
  const platform = "runtime:node -p process.platform";
  const platformReaders = cached.filter((id) => initial[id].nodes[platform] !== undefined);
  const preload = join(scratch, "other-platform.cjs");
  writeFileSync(
    preload,
    'if (process.execArgv.includes("process.platform")) Object.defineProperty(process, "platform", {value: "nx-other-platform"});\n',
  );
  const otherPlatform = hashes(dir, platformReaders, { NODE_OPTIONS: `--require=${preload}` });
  requireMoved(initial, otherPlatform, platformReaders, "process.platform");
  for (const id of platformReaders)
    assert.notEqual(initial[id].nodes[platform], otherPlatform[id].nodes[platform]);

  const nxPath = join(dir, "nx.json");
  const originalNx = readFileSync(nxPath, "utf8");
  const toolchain = join(dir, "rust-toolchain.toml");
  const originalToolchain = readFileSync(toolchain, "utf8");
  definition.namedInputs.sharedGlobals = definition.namedInputs.sharedGlobals.filter(
    (input) => input !== "{workspaceRoot}/rust-toolchain.toml",
  );
  writeFileSync(nxPath, JSON.stringify(definition, null, 2));
  try {
    const regressed = hashes(dir, ["onetaskgraph-core:test"]);
    writeFileSync(toolchain, originalToolchain.replace('channel = "1.97.1"', 'channel = "1.97.0"'));
    const stale = hashes(dir, ["onetaskgraph-core:test"]);
    assert.throws(
      () => requireMoved(regressed, stale, ["onetaskgraph-core:test"], "rust-toolchain.toml"),
      /lost input rust-toolchain\.toml/,
    );
  } finally {
    writeFileSync(toolchain, originalToolchain);
    writeFileSync(nxPath, originalNx);
  }
});

test("raw text readers invalidate on formatting the semantic digest intentionally discards", () => {
  const dir = fixture("raw-formatting", "HEAD", true);
  const ids = ["sdk-typescript:format-check", "sdk-typescript:typecheck", "workspace:typecheck"];
  const initial = hashes(dir, ids);
  const sdkPath = join(dir, "sdks/typescript/package.json");
  const sdk = readFileSync(sdkPath, "utf8");
  const formatter = initial["sdk-typescript:format-check"].definition.options.command;
  assert.equal(typeof formatter, "string");
  command(dir, "bash", ["-c", formatter]);
  writeFileSync(sdkPath, ` ${sdk}`);
  try {
    const changed = hashes(dir, ids);
    requireMoved(initial, changed, ["sdk-typescript:format-check"], "raw package.json formatting");
    assert.equal(
      changed["sdk-typescript:typecheck"].hash,
      initial["sdk-typescript:typecheck"].hash,
    );
    const refused = spawnSync("bash", ["-c", formatter], {
      cwd: dir,
      env: environment,
      encoding: "utf8",
    });
    assert.equal(refused.status, 1, refused.stdout + refused.stderr);
    assert.match(refused.stdout + refused.stderr, /sdks\/typescript\/package\.json/);
  } finally {
    writeFileSync(sdkPath, sdk);
  }
  command(dir, "bash", ["scripts/check-workspace-config.sh"]);
  const lockPath = join(dir, "bun.lock");
  const lock = readFileSync(lockPath, "utf8");
  writeFileSync(
    lockPath,
    lock
      .replaceAll('"version": "', '"version":  "')
      .replaceAll('"@onetaskgraph/cli": "', '"@onetaskgraph/cli":  "'),
  );
  try {
    const changed = hashes(dir, ids);
    requireMoved(initial, changed, ["workspace:typecheck"], "raw Bun version-line formatting");
    assert.equal(
      changed["sdk-typescript:typecheck"].hash,
      initial["sdk-typescript:typecheck"].hash,
    );
    const refused = spawnSync("bash", ["scripts/check-workspace-config.sh"], {
      cwd: dir,
      env: environment,
      encoding: "utf8",
    });
    assert.equal(refused.status, 1, refused.stdout + refused.stderr);
    assert.match(refused.stdout + refused.stderr, /bun\.lock: no product version could be read/);
  } finally {
    writeFileSync(lockPath, lock);
  }
});

test("both GitHub Projects targets include run and attempt nonces on every OS", () => {
  const dir = fixture("github-nonces", "HEAD", true);
  const ids = ["onetaskgraph-github-projects:test", "onetaskgraph-github-projects-e2e:test"];
  const initial = hashes(dir, ids);
  for (const [key, value] of [
    ["GITHUB_RUN_ID", "1002"],
    ["GITHUB_RUN_ATTEMPT", "2"],
  ]) {
    const changed = hashes(dir, ids, { [key]: value });
    requireMoved(initial, changed, ids, key);
    for (const id of ids)
      assert.ok(
        initial[id].definition.inputs.some((input) => input?.env === key),
        `${id}: direct ${key} input`,
      );
  }
});

test("real Nx preserves baseline affected routing for every manifest and lockfile, including release-only edits", () => {
  const before = fixture("selection-base", baseline);
  const after = fixture("selection-current", "HEAD", true);
  const names = new Set(Object.values(canonicalInputNames()).flat());
  const manifests = command(before, "git", ["ls-files"])
    .trim()
    .split("\n")
    .filter((path) => names.has(path.split("/").at(-1)));
  const releaseFiles = command(root, "git", ["diff", "--name-only", prerelease, baseline])
    .trim()
    .split("\n");
  function affected(dir, paths) {
    rmSync(join(dir, ".nx"), { recursive: true, force: true });
    const projects = JSON.parse(
      command(dir, process.execPath, [
        "node_modules/.bin/nx",
        "show",
        "projects",
        "--affected",
        `--files=${paths.join(",")}`,
        "--json",
      ]),
    );
    assert.ok(Array.isArray(projects) && projects.every((name) => typeof name === "string"));
    return new Set(projects);
  }
  // Nx routes changed paths before hashing their contents. Dependency and feature
  // mutations are checked above; here every canonicalized path must keep that routing.
  for (const paths of [...manifests.map((path) => [path]), releaseFiles]) {
    const expected = affected(before, paths);
    const actual = affected(after, paths);
    for (const project of expected)
      assert.ok(actual.has(project), `${paths.join(",")}: lost affected project ${project}`);
  }
});
