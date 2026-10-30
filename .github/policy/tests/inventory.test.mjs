import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import {
  cdJourneyFiles,
  GITHUB_READS,
  inputProblems,
  policyJourneyFiles,
  policyJourneyReadSet,
  rustCacheJourneyFiles,
} from "../../test-inputs.mjs";
import { parse } from "yaml";
import { graphReadSet, read } from "../../workflow-graph.mjs";

// This project's inputs are every .github file, so an edit on either side of these
// reconciliations — this project's command, github-workflows', test-inputs.mjs or a test file
// added to either directory — selects it, where github-workflows' own ci-contexts would not see
// an edit under policy/.
const load = (file) => JSON.parse(readFileSync(new URL(file, import.meta.url), "utf8"));

test("the policy journeys are keyed on the combined read set of the tests they run", () => {
  assert.deepEqual(inputProblems(load("../project.json"), "policy-tests"), []);
});

test("an undeclared read in the policy journey target is refused", () => {
  const graph = graphReadSet().filter((path) => !path.startsWith(".github/"));
  for (const path of new Set([...graph, ...policyJourneyReadSet, ".github/**/*"])) {
    const project = load("../project.json");
    for (const [name, inputs] of Object.entries(project.namedInputs))
      project.namedInputs[name] = inputs.filter((input) => input !== `{workspaceRoot}/${path}`);
    assert.ok(
      inputProblems(project, "policy-tests").includes(`policy-tests is not keyed on ${path}`),
      path,
    );
  }
  assert.equal(GITHUB_READS, "{workspaceRoot}/.github/**/*");
});

test("the two projects run every journey exactly once through its owning target", () => {
  const workflows = load("../../project.json");
  const policy = load("../project.json");
  for (const [project, target, files] of [
    [policy, "policy-tests", policyJourneyFiles],
    [policy, "rust-cache-action", rustCacheJourneyFiles],
    [workflows, "cd-journeys", cdJourneyFiles],
  ]) {
    assert.equal(project.targets.test.executor, "nx:noop");
    assert.ok(project.targets.test.dependsOn.includes(target));
    assert.equal(project.targets[target].options.command, `node --test ${files.join(" ")}`);
  }
  assert.equal(workflows.targets["policy-tests"], undefined);
  // Every policy journey sits under this project's root, which github-workflows' own inputs
  // leave out, so editing one alone does not select the CD journeys.
  assert.ok(workflows.namedInputs.default.includes("!{projectRoot}/policy/**/*"));
  for (const file of [...policyJourneyFiles, ...rustCacheJourneyFiles])
    assert.ok(file.startsWith(".github/policy/"), file);
  const actual = ["tests", "policy/tests"].flatMap((directory) =>
    readdirSync(new URL(`../../${directory}/`, import.meta.url))
      .filter((file) => file.endsWith(".test.mjs"))
      .map((file) => `.github/${directory}/${file}`),
  );
  assert.deepEqual(
    [...policyJourneyFiles, ...rustCacheJourneyFiles, ...cdJourneyFiles].sort(),
    actual.sort(),
  );
});

// Selection by real Nx, which is what the split is for: a crate edit selects the CD journeys
// and not the policy journeys, a policy edit the reverse, and a workflow edit both.
test("a crate edit selects the CD journeys without the policy journeys", () => {
  const require = createRequire(import.meta.url);
  const nx = resolve(
    dirname(require.resolve("nx/package.json")),
    require("nx/package.json").bin.nx,
  );
  const cwd = fileURLToPath(new URL("../../../", import.meta.url));
  const selected = (file) =>
    JSON.parse(
      execFileSync(
        process.execPath,
        [nx, "show", "projects", "--affected", "--files", file, "--json"],
        { cwd, encoding: "utf8" },
      ),
    );
  for (const [file, workflows, policy] of [
    ["crates/onetaskgraph-core/src/lib.rs", true, false],
    ["sdks/python/project.json", true, false],
    [".github/policy/tests/policy-routing.test.mjs", false, true],
    [".github/policy/project.json", false, true],
    [".github/policy/budgets.yaml", false, true],
    [".github/test-inputs.mjs", true, true],
    [".github/tests/release-cd.test.mjs", true, true],
    [".github/workflows/ci.yml", true, true],
  ]) {
    const projects = selected(file);
    assert.equal(projects.includes("github-workflows"), workflows, `${file}: github-workflows`);
    assert.equal(
      projects.includes("github-workflow-policy"),
      policy,
      `${file}: github-workflow-policy`,
    );
  }
});

// A release guard may skip Windows only where what it drives runs on Linux alone in production;
// then it must still run in full, unconditionally, in test-linux. Derived from
// scripts:distribution-check rather than listed, so a guard that starts skipping is held too.
const WINDOWS = ["*Windows_NT*", "*msys*", "*cygwin*", "*win32*"];
const MOVED_OFF_WINDOWS = [
  "scripts/check-release-pr-sync.sh",
  "scripts/check-release-tooling-selection.sh",
  "scripts/check-crate-sibling-resolution.sh",
  "scripts/check-crate-sibling-resolution-same-second.sh",
];

function windowsSkips(sources) {
  const scripts = JSON.parse(sources["scripts/project.json"]);
  const options = scripts.targets["distribution-check"].options;
  return (options.commands ?? [options.command])
    .map((command) => command.match(/^bash (scripts\/check-[a-z0-9-]+\.sh)$/)?.[1])
    .filter((guard) => guard && /skipped on Windows/.test(sources[guard]));
}

function skipProblems(sources) {
  const problems = [];
  for (const guard of windowsSkips(sources)) {
    const skip = sources[guard].match(
      /case "\$\{OS:-\}\$\{OSTYPE:-\}" in\n\s*([^\n]+)\)\n[^\n]*skipped on Windows[^\n]*\n\s*exit 0/,
    );
    if (!skip) {
      problems.push(`${guard} skips Windows in a shape this inventory cannot read`);
      continue;
    }
    for (const pattern of skip[1].split("|").map((alternative) => alternative.trim()))
      if (!WINDOWS.includes(pattern)) problems.push(`${guard} also skips ${pattern}`);
  }
  const ci = parse(sources[".github/workflows/ci.yml"]);
  const job = ci.jobs["test-linux"];
  if (job?.["runs-on"] !== "ubuntu-latest")
    problems.push("test-linux does not run on ubuntu-latest");
  if (!job?.steps?.some((step) => step.run === "just ci-test-linux"))
    problems.push("test-linux does not run `just ci-test-linux`");
  if (!/\nci-test-linux[^\n]*:\n[^\n]*scripts\/ci-job\.sh test-linux\b/.test(sources.justfile))
    problems.push("`just ci-test-linux` does not run scripts/ci-job.sh test-linux");
  const arm = sources["scripts/ci-job.sh"].match(
    /\n {2}test-linux\)\n[\s\S]*?phases="([\s\S]*?)"\n/,
  );
  if (!arm?.[1].split("\n").includes("1 run scripts:distribution-check -"))
    problems.push("test-linux's phases do not run scripts:distribution-check unconditionally");
  return problems;
}

function guardSources() {
  const paths = [
    "scripts/project.json",
    ".github/workflows/ci.yml",
    "justfile",
    "scripts/ci-job.sh",
  ];
  const sources = Object.fromEntries(paths.map((path) => [path, read(path)]));
  const scripts = JSON.parse(sources["scripts/project.json"]);
  for (const command of scripts.targets["distribution-check"].options.commands) {
    const guard = command.match(/^bash (scripts\/check-[a-z0-9-]+\.sh)$/)?.[1];
    if (guard) sources[guard] = read(guard);
  }
  return sources;
}

test("every release guard that skips Windows runs in full in test-linux", () => {
  const sources = guardSources();
  for (const guard of MOVED_OFF_WINDOWS) assert.ok(windowsSkips(sources).includes(guard), guard);
  assert.deepEqual(skipProblems(sources), []);
});

test("a skip widened past Windows, or test-linux leaving distribution-check, is refused", () => {
  const original = guardSources();
  const refused = (edit, expected) => {
    const sources = structuredClone(original);
    edit(sources);
    assert.ok(
      skipProblems(sources).some((problem) => expected.test(problem)),
      `${expected}: ${JSON.stringify(skipProblems(sources))}`,
    );
  };
  refused((sources) => {
    const guard = MOVED_OFF_WINDOWS[0];
    sources[guard] = sources[guard].replace("*win32*)", "*win32* | *linux*)");
  }, /check-release-pr-sync\.sh also skips \*linux\*/);
  refused((sources) => {
    sources["scripts/ci-job.sh"] = sources["scripts/ci-job.sh"].replace(
      /(\n {2}test-linux\)[\s\S]*?)1 run scripts:distribution-check -\n/,
      (_, before) => before,
    );
  }, /do not run scripts:distribution-check unconditionally/);
  refused((sources) => {
    sources[".github/workflows/ci.yml"] = sources[".github/workflows/ci.yml"].replace(
      /(\n {2}test-linux:\n(?: {4}[^\n]*\n)*? {4}runs-on: )ubuntu-latest/,
      "$1windows-latest",
    );
  }, /test-linux does not run on ubuntu-latest/);
});

// Which projects a job's phases run a target for, as Nx reads each phase's exclusions: a name,
// a `tag:`, `*`, and `!name` putting one back, in order.
function selects(job, target, project, tags, ciJob) {
  const arm = ciJob.match(new RegExp(`\\n {2}${job}\\)\\n[\\s\\S]*?phases="([\\s\\S]*?)"\\n`));
  if (!arm) return false;
  return arm[1].split("\n").some((line) => {
    const [, kind, targets, excluded = "-"] = line.trim().split(/\s+/);
    if (kind !== "affected" || !targets.split(",").includes(target)) return false;
    let out = false;
    for (const pattern of excluded === "-" ? [] : excluded.split(",")) {
      if (pattern === "*") out = true;
      else if (pattern === `!${project}`) out = false;
      else if (
        pattern === project ||
        (pattern.startsWith("tag:") && tags.includes(pattern.slice(4)))
      )
        out = true;
    }
    return !out;
  });
}

function placementProblems(ciJob) {
  const problems = [];
  const tagsOf = (file) => load(file).tags ?? [];
  const suites = [
    ["github-workflows", "test", tagsOf("../../project.json")],
    ["github-workflow-policy", "test", tagsOf("../project.json")],
  ];
  for (const [project, target, tags] of suites) {
    if (!selects("test-linux", target, project, tags, ciJob))
      problems.push(`test-linux does not run ${project}:${target}`);
    for (const job of ["test-windows", "test-macos"])
      if (selects(job, target, project, tags, ciJob))
        problems.push(`${job} runs ${project}:${target}, whose suites model Linux jobs`);
  }
  const policyTags = tagsOf("../project.json");
  for (const job of ["test-linux", "test-windows", "test-macos"])
    if (
      !selects(job, "rust-cache-action", "github-workflow-policy", policyTags, ciJob) &&
      !(job === "test-linux" && selects(job, "test", "github-workflow-policy", policyTags, ciJob))
    )
      problems.push(`${job} does not run the real rust-cache action's journey`);
  return problems;
}

test("the workflow suites run in test-linux, and the rust-cache journey on every runner", () => {
  assert.deepEqual(placementProblems(read("scripts/ci-job.sh")), []);
  assert.deepEqual(load("../project.json").targets.test.dependsOn.sort(), [
    "policy-tests",
    "rust-cache-action",
  ]);
});

test("a workflow suite leaving test-linux, or the rust-cache journey leaving a runner, is refused", () => {
  const ciJob = read("scripts/ci-job.sh");
  const refused = (edited, expected) =>
    assert.ok(
      placementProblems(edited).some((problem) => expected.test(problem)),
      `${expected}: ${JSON.stringify(placementProblems(edited))}`,
    );
  refused(
    ciJob.replace(
      "1 affected test tag:test-in-coverage:*,onetaskgraph-linear,onetaskgraph-linear-e2e\n",
      "1 affected test tag:test-in-coverage:*,onetaskgraph-linear,onetaskgraph-linear-e2e,github-workflows\n",
    ),
    /test-linux does not run github-workflows:test/,
  );
  refused(
    ciJob.replace(
      /(\n {2}test-windows\)[\s\S]*?)1 affected rust-cache-action -\n/,
      (_, before) => before,
    ),
    /test-windows does not run the real rust-cache action's journey/,
  );
  refused(
    ciJob.replace(
      /(\n {2}test-macos\)[\s\S]*?),github-workflows,github-workflow-policy --parallel=2/,
      (_, before) => `${before} --parallel=2`,
    ),
    /test-macos runs github-workflows:test/,
  );
});
