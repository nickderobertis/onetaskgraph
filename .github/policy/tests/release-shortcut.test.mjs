import "../../test-setup.mjs";
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { after, test } from "node:test";
import { fileURLToPath } from "node:url";
import { fixtureEnvironment } from "../../fixture-environment.mjs";
import { read, root } from "../../workflow-graph.mjs";

// The release-PR shortcut, driven as the three test jobs run it, over a real scratch
// repository whose `main` holds the release-owned manifests a release-plz pull request bumps.
const scratchRoot = process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir();
const repository = mkdtempSync(resolve(scratchRoot, "release-shortcut-"));
const program = resolve(repository, ".github/release-shortcut.mjs");
after(() => rmSync(repository, { recursive: true, force: true }));
const golden = JSON.parse(read(".github/policy/tests/fixtures/release-coverage-v1.json"));

function git(...args) {
  return execFileSync("git", args, { cwd: repository, encoding: "utf8" }).trim();
}
function put(path, text) {
  const destination = resolve(repository, path);
  mkdirSync(resolve(destination, ".."), { recursive: true });
  writeFileSync(destination, text);
}
function commit(message, changes = {}) {
  for (const [path, text] of Object.entries(changes)) put(path, text);
  git("add", ".");
  git("commit", "-qm", message);
  return git("rev-parse", "HEAD");
}

git("init", "-q", "-b", "main");
git("config", "user.email", "shortcut@example.invalid");
git("config", "user.name", "Release shortcut");
put(".gitignore", "github-output\n.github/node_modules/\n");
for (const file of ["release-shortcut.mjs", "semantic-release.py", "semantic-json.mjs"])
  put(`.github/${file}`, read(`.github/${file}`));
put("scripts/product_versions.py", read("scripts/product_versions.py"));
symlinkSync(
  fileURLToPath(new URL(".github/node_modules", root)),
  resolve(repository, ".github/node_modules"),
  process.platform === "win32" ? "junction" : "dir",
);

const released = (version) => ({
  "Cargo.toml": `[workspace]\nmembers = ["crates/*"]\n[workspace.package]\nversion = "${version}"\n[workspace.dependencies]\nonetaskgraph-core = { path = "crates/onetaskgraph-core", version = "=${version}" }\n`,
  "crates/onetaskgraph-core/Cargo.toml": `[package]\nname = "onetaskgraph-core"\nversion = "${version}"\n`,
  "Cargo.lock": `version = 4\n[[package]]\nname = "onetaskgraph-core"\nversion = "${version}"\n[[package]]\nname = "external"\nversion = "1.0.0"\nsource = "registry+example"\nchecksum = "abc"\n`,
  "pyproject.toml": `[project]\nname = "onetaskgraph-cli"\nversion = "${version}"\n`,
  "sdks/python/pyproject.toml": `[project]\nname = "onetaskgraph-sdk"\nversion = "${version}"\ndependencies = ["onetaskgraph-cli==${version}"]\n`,
});
const source = (body) => ({
  "crates/onetaskgraph-core/src/lib.rs": `pub fn answer() -> u32 { ${body} }\n`,
});

commit("chore: first release", {
  ...released("1.2.3"),
  ...source("41"),
  "CHANGELOG.md": "# Changes\n",
});
const mainTip = commit("fix: the answer", source("42"));
// What release-plz opens: main's tip plus the bump and the changelog entry, nothing else.
git("checkout", "-q", "-b", "release-plz-2026-10-16");
const release = commit("chore: release v1.2.4", {
  ...released("1.2.4"),
  "CHANGELOG.md": "# Changes\n\n## 1.2.4\n\n- the answer\n",
});
const releasePlusSource = commit("chore: release v1.2.4 and a fix", source("43"));
// A base main never passed: a commit only ever on a side branch, with a release on top.
git("checkout", "-q", "-b", "unmerged", mainTip);
const unmerged = commit("feat: never merged", source("44"));
const releaseOnUnmerged = commit("chore: release v1.2.4", released("1.2.4"));
git("checkout", "-q", "main");

function decide(base, head, extra = [], via = program) {
  const output = resolve(repository, "github-output");
  rmSync(output, { force: true });
  const result = spawnSync(
    process.execPath,
    [via, "decide", "--base", base, "--head", head, "--main", "main", ...extra],
    { cwd: repository, encoding: "utf8", env: { ...fixtureEnvironment(), GITHUB_OUTPUT: output } },
  );
  return { ...result, output: result.status === 0 ? readFileSync(output, "utf8") : "" };
}
function coverage(base, head) {
  const result = decide(base, head);
  assert.equal(result.status, 0, result.stderr);
  const decision = JSON.parse(result.stdout);
  assert.equal(
    result.output,
    `rust_coverage=${decision.rust_coverage}\nreason=${decision.reason}\n`,
  );
  return decision;
}

test("a tree main passed plus only release-plz's version and changelog edits skips only Rust coverage", () => {
  const decision = coverage(mainTip, release);
  assert.deepEqual(decision, golden.skip);
  assert.match(decision.reason, /main passed plus only release-plz's version and changelog edits/);
});

test("the program decides when it is reached through a symlinked directory", () => {
  // As macOS's temporary directory is: a run that decided nothing would read as a pass.
  const link = `${repository}-link`;
  rmSync(link, { force: true, recursive: true });
  symlinkSync(repository, link, process.platform === "win32" ? "junction" : "dir");
  try {
    const result = decide(mainTip, release, [], resolve(link, ".github/release-shortcut.mjs"));
    assert.equal(result.status, 0, result.stderr);
    assert.equal(JSON.parse(result.stdout).rust_coverage, "skip");
    assert.match(result.output, /^rust_coverage=skip\n/);
  } finally {
    rmSync(link, { force: true, recursive: true });
  }
});

test("a release head carrying any other change runs Rust coverage", () => {
  const decision = coverage(mainTip, releasePlusSource);
  assert.deepEqual(decision, golden.source);
  assert.match(decision.reason, /changes more than release-owned versions/);
  // A source change alone, with no release edit at all, is not a release either.
  git("checkout", "-q", "-b", "feature", mainTip);
  const feature = commit("fix: another answer", source("45"));
  git("checkout", "-q", "main");
  assert.equal(coverage(mainTip, feature).rust_coverage, "run");
  // Nor is an empty diff: nothing was released, so nothing was proven.
  assert.equal(coverage(mainTip, mainTip).rust_coverage, "run");
});

test("a release on a base tree main never passed runs Rust coverage", () => {
  const decision = coverage(unmerged, releaseOnUnmerged);
  assert.equal(decision.rust_coverage, "run");
  assert.match(decision.reason, /base tree is not one main passed/);
});

test("an unreadable ref runs Rust coverage, and a malformed invocation fails rather than skipping", () => {
  assert.deepEqual(coverage("0".repeat(40), release), golden.unreadable);
  assert.equal(coverage(mainTip, "no-such-ref").rust_coverage, "run");
  const malformed = decide(mainTip, release, ["--extra"]);
  assert.equal(malformed.status, 1);
  assert.match(malformed.stderr, /usage: release-shortcut\.mjs decide/);
  assert.equal(malformed.output, "");
});

test("an unreadable main, or one spelled as an option, runs Rust coverage", () => {
  // `--all` would read every branch's history, the unmerged one's included, as main's.
  for (const main of ["no-such-branch", "--all", "--branches"]) {
    const result = spawnSync(
      process.execPath,
      [program, "decide", "--base", unmerged, "--head", releaseOnUnmerged, "--main", main],
      { cwd: repository, encoding: "utf8" },
    );
    assert.equal(result.status, 0, result.stderr);
    assert.equal(JSON.parse(result.stdout).rust_coverage, "run", main);
  }
});

test("a comparator that fails, prints malformed JSON or answers other than a boolean runs Rust coverage", () => {
  const comparator = resolve(repository, ".github/semantic-release.py");
  const real = readFileSync(comparator, "utf8");
  try {
    for (const body of [
      "raise SystemExit(1)\n",
      "print('{not json')\n",
      "print('\"true\"')\n",
      "print('null')\n",
    ]) {
      writeFileSync(comparator, body);
      const decision = coverage(mainTip, release);
      assert.equal(decision.rust_coverage, "run", body);
      assert.match(decision.reason, /Unreadable release diff/);
    }
  } finally {
    writeFileSync(comparator, real);
  }
  assert.equal(coverage(mainTip, release).rust_coverage, "skip");
});

test("a tree only a merged branch held, never main's own, runs Rust coverage", () => {
  // A merge brings a branch's commits into main's history as second parents, but only the
  // merge itself is a tree main's checks passed; a release cut from one of those commits
  // has not been proven by main.
  git("checkout", "-q", "-b", "merged", mainTip);
  const sideTip = commit("feat: merged later", source("46"));
  // Main moves on before the merge, so the merge's tree is not the side branch's.
  git("checkout", "-q", "main");
  commit("docs: main's own change", { "docs/notes.md": "main moved on\n" });
  git("merge", "-q", "--no-ff", "-m", "Merge the side branch", "merged");
  git("checkout", "-q", "-b", "release-on-side", sideTip);
  const releaseOnSide = commit("chore: release v1.2.4", released("1.2.4"));
  git("checkout", "-q", "main");
  const decision = coverage(sideTip, releaseOnSide);
  assert.equal(decision.rust_coverage, "run");
  assert.match(decision.reason, /base tree is not one main passed/);
  // The merge itself is main's own, so a release on it still skips.
  const merge = git("rev-parse", "main");
  git("checkout", "-q", "-b", "release-on-merge", merge);
  const releaseOnMerge = commit("chore: release v1.2.4", released("1.2.4"));
  git("checkout", "-q", "main");
  assert.equal(coverage(merge, releaseOnMerge).rust_coverage, "skip");
});

// A project's definition, by the name its project.json gives it.
function projectNamed(project) {
  const files = execFileSync("git", ["ls-files", "*project.json"], {
    cwd: fileURLToPath(root),
    encoding: "utf8",
  })
    .split("\n")
    .filter(Boolean);
  const definition = files
    .map((file) => JSON.parse(read(file)))
    .find((candidate) => candidate.name === project);
  assert.ok(definition, `${project}: a project.json names it`);
  return definition;
}
const recordsTelemetryUnderCoverage = (project) =>
  (projectNamed(project).targets?.coverage?.outputs ?? []).some((output) =>
    output.includes("/target/debug/telemetry/"),
  );

test("the release coverage switch keeps functional and SDK tasks and omits only Rust coverage telemetry consumers", () => {
  const select = (skip) => {
    const result = spawnSync("bash", ["scripts/ci-job.sh", "test-linux", "--select-only"], {
      cwd: fileURLToPath(root),
      env: {
        ...fixtureEnvironment(),
        GIT_DIR: undefined,
        GIT_WORK_TREE: undefined,
        GIT_INDEX_FILE: undefined,
        GIT_COMMON_DIR: undefined,
        NX_DAEMON: "false",
        NX_BASE: "143002fe022eca174b58ccac06497800aedc9092",
        NX_HEAD: "HEAD",
        ONETASKGRAPH_SKIP_RUST_COVERAGE: skip,
      },
      encoding: "utf8",
    });
    assert.equal(result.status, 0, result.stdout + result.stderr);
    const selected = result.stdout
      .match(/ci-job: test-linux: selects ([^\n]+)/)?.[1]
      .trim()
      .split(/\s+/);
    assert.ok(selected?.length);
    return selected;
  };
  const normal = select("0");
  const release = select("1");
  const budgets = [
    "onetaskgraph-e2e:budgets",
    "onetaskgraph-github-projects-e2e:budgets",
    "onetaskgraph-linear-e2e:budgets",
    "onetaskgraph-live-janitor:budgets",
  ];
  for (const task of [
    "onetaskgraph-core:coverage",
    "onetaskgraph-core:test-uncovered",
    ...budgets,
  ]) {
    assert.ok(normal.includes(task), `${task}: normally selected`);
    assert.ok(!release.includes(task), `${task}: omitted with Rust coverage`);
  }
  for (const task of [
    "sdk-python:coverage",
    "sdk-typescript:coverage",
    "github-workflow-policy:budgets",
    "onetaskgraph-github-projects:test",
    // scripts:distribution-test is lint's, which the switch does not reach.
    "scripts:distribution-check",
  ])
    assert.ok(release.includes(task), `${task}: still selected on release`);
  // test-linux runs budgets without their dependencies, so a budget whose telemetry its
  // project's `coverage` records has nothing to read unless that coverage runs beside it. A
  // release tree went red once such a budget was added and the switch still selected it.
  for (const [mode, selected] of [
    ["normal", normal],
    ["release", release],
  ])
    for (const task of selected.filter((name) => name.endsWith(":budgets"))) {
      const project = task.slice(0, -":budgets".length);
      if (recordsTelemetryUnderCoverage(project))
        assert.ok(
          selected.includes(`${project}:coverage`),
          `${task}: selected on ${mode} without ${project}:coverage, which records its telemetry`,
        );
    }
  const omitted = normal.filter((task) => !release.includes(task));
  assert.ok(omitted.length > budgets.length);
  for (const task of omitted)
    assert.ok(
      budgets.includes(task) ||
        /:(coverage|coverage-build|test-uncovered)$/.test(task) ||
        // A test-in-coverage crate's `test` is in the plan only as its budget's dependency,
        // which test-linux never runs, so it goes with that budget.
        (task.endsWith(":test") &&
          budgets.includes(task.replace(/:test$/, ":budgets")) &&
          projectNamed(task.replace(/:test$/, "")).tags?.includes("test-in-coverage:crate")),
      `${task}: functional work was omitted`,
    );
});

test("the coverage switch refuses malformed values and use on an uninstrumented OS leg", () => {
  for (const [job, value] of [
    ["test-linux", "true"],
    ["test-windows", "1"],
    ["test-macos", "1"],
  ]) {
    const result = spawnSync("bash", ["scripts/ci-job.sh", job, "--select-only"], {
      cwd: fileURLToPath(root),
      env: { ...fixtureEnvironment(), ONETASKGRAPH_SKIP_RUST_COVERAGE: value },
      encoding: "utf8",
    });
    assert.equal(result.status, 2, result.stdout + result.stderr);
    assert.match(result.stderr, /ONETASKGRAPH_SKIP_RUST_COVERAGE/);
  }
});
