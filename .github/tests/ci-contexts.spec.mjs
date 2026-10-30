import {
  cdJourneyReadSet,
  policyJourneyReadSet,
  cdJourneyFiles,
  policyJourneyFiles,
} from "../test-inputs.mjs";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { graph, graphReadSet } from "../workflow-graph.mjs";
import { definitions, refusals } from "./workflow-definitions.mjs";

// Today's required contexts, as the run graph resolves a pull request's runs. The graph reads
// files outside .github (graphReadSet() names them), so this is the github-workflows project's
// `ci-contexts` target, keyed on those files too, rather than the .github-only `ci-jobs`.

// llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] Branch protection's required set is not this repository's to hold: its source is the create-repo skill's setup_github_governance.py, and the orchestration repository reconciles its inventory of this merge path against GitHub on every gate run (AGENTS.md, "Commits, releases, and merging"). A pull request's token cannot read branch protection, so this is the set the workflow must keep producing, held to the workflow below.
const REQUIRED_CONTEXTS = [
  "check (ubuntu-latest)",
  "check (windows-latest)",
  "deny",
  "llmlint",
  "pr-title",
  "install path (ubuntu-latest)",
  "install path (windows-latest)",
  "visual-docs / report (x86_64, shots/current, shots/verify, Visual docs)",
];
// llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]

// Every check context a source pull request's runs report, resolved as workflow-graph.mjs
// resolves a run: reusable workflows through their locked definitions, every matrix
// dimension, each job's display name. A tree it cannot resolve reports none.
function reportedOnPullRequest(all) {
  const scenario = {
    event: "pull_request",
    action: "synchronize",
    headRef: "feature",
    fork: false,
  };
  try {
    return graph(scenario, { macosRequired: true, reason: "a source change" }, all).flatMap((run) =>
      run.rows.map((row) => row.name),
    );
  } catch {
    return [];
  }
}

// Today's required contexts keep a job behind each of them until protection moves.
export function problems(all) {
  const reported = reportedOnPullRequest(all);
  return REQUIRED_CONTEXTS.filter((context) => !reported.includes(context)).map(
    (context) => `no job reports the required context ${context}`,
  );
}
const refuses = refusals(problems);

test("the finished workflows report every one of today's required contexts", () => {
  assert.deepEqual(problems(definitions()), []);
});

test("a job behind one of today's required contexts going missing is refused", () => {
  const removals = {
    "check (ubuntu-latest)": (all) => {
      all["ci.yml"].jobs.check.strategy.matrix.os = ["windows-latest"];
    },
    "check (windows-latest)": (all) => {
      all["ci.yml"].jobs.check.strategy.matrix.os = ["ubuntu-latest"];
    },
    deny: (all) => delete all["ci.yml"].jobs.deny,
    llmlint: (all) => delete all["ci.yml"].jobs.llmlint,
    "pr-title": (all) => delete all["ci.yml"].jobs["pr-title"],
    "install path (ubuntu-latest)": (all) => delete all["ci.yml"].jobs["install-path"],
    "install path (windows-latest)": (all) => {
      all["ci.yml"].jobs["install-path"].strategy.matrix.os = ["ubuntu-latest"];
    },
    "visual-docs / report (x86_64, shots/current, shots/verify, Visual docs)": (all) =>
      delete all["visual-docs.yml"].jobs["visual-docs"],
  };
  assert.deepEqual(Object.keys(removals).sort(), [...REQUIRED_CONTEXTS].sort());
  for (const [context, remove] of Object.entries(removals))
    refuses(remove, new RegExp(`required context ${context.replace(/[()/]/g, "\\$&")}`));
  refuses((all) => {
    all["visual-docs.yml"].jobs["visual-docs"].name = "Visual docs";
  }, /required context visual-docs/);
});

// The policy journeys are github-workflow-policy's (.github/policy/project.json), split out so a
// crate edit, which selects the CD journeys, does not run them; every other target here is
// github-workflows'.
const PROJECT_OF = {
  "ci-contexts": "../project.json",
  "cd-journeys": "../project.json",
  "policy-tests": "../policy/project.json",
};
const load = (file) => JSON.parse(readFileSync(new URL(file, import.meta.url), "utf8"));
// github-workflow-policy is rooted beside, not at, .github, so it names the .github files it
// reads as one workspace glob; that glob is its own reads rather than an outside one.
const GITHUB_READS = "{workspaceRoot}/.github/**/*";

// Each executable target declares only the external read set its journeys consume.
function inputProblems(project, target) {
  const expand = (inputs) =>
    inputs.flatMap((input) =>
      typeof input === "string" && project.namedInputs?.[input]
        ? expand(project.namedInputs[input])
        : [input],
    );
  const graph = graphReadSet().filter((path) => !path.startsWith(".github/"));
  assert.ok(graph.length > 0, "the graph reads no outside file, so this proves nothing");
  const outside = [
    ...new Set([
      ...graph,
      ...(target === "cd-journeys"
        ? cdJourneyReadSet
        : target === "policy-tests"
          ? policyJourneyReadSet
          : []),
    ]),
  ];
  const inputs = expand(project.targets[target].inputs ?? ["default"]);
  const github = target === "policy-tests" ? [GITHUB_READS] : [];
  return [
    ...github
      .filter((entry) => !inputs.includes(entry))
      .map((entry) => `${target} is not keyed on ${entry.slice("{workspaceRoot}/".length)}`),
    ...outside
      .filter((path) => !inputs.includes(`{workspaceRoot}/${path}`))
      .map((path) => `${target} is not keyed on ${path}`),
    ...inputs
      .filter((entry) => String(entry).startsWith("{workspaceRoot}/"))
      .filter((input) => !github.includes(input))
      .filter((input) => !outside.includes(input.slice("{workspaceRoot}/".length)))
      .map((input) => `${target} is keyed on ${input}, which its tests do not read`),
  ];
}

test("each target is keyed on the combined read set of the tests it runs", () => {
  for (const [target, file] of Object.entries(PROJECT_OF))
    assert.deepEqual(inputProblems(load(file), target), []);
});

test("an undeclared read in either journey target is refused", () => {
  const graph = graphReadSet().filter((path) => !path.startsWith(".github/"));
  for (const [target, reads] of [
    ["policy-tests", policyJourneyReadSet],
    ["cd-journeys", cdJourneyReadSet],
  ]) {
    for (const path of new Set([...graph, ...reads, ".github/**/*"])) {
      if (path === ".github/**/*" && target !== "policy-tests") continue;
      const project = load(PROJECT_OF[target]);
      for (const [name, inputs] of Object.entries(project.namedInputs))
        project.namedInputs[name] = inputs.filter((input) => input !== `{workspaceRoot}/${path}`);
      assert.ok(
        inputProblems(project, target).includes(`${target} is not keyed on ${path}`),
        `${target}: ${path}`,
      );
    }
  }
});

test("the two projects run every journey exactly once through its owning target", () => {
  const workflows = load("../project.json");
  const policy = load("../policy/project.json");
  for (const [project, target, files] of [
    [policy, "policy-tests", policyJourneyFiles],
    [workflows, "cd-journeys", cdJourneyFiles],
  ]) {
    assert.equal(project.targets.test.executor, "nx:noop");
    assert.ok(project.targets.test.dependsOn.includes(target));
    assert.equal(project.targets[target].options.command, `node --test ${files.join(" ")}`);
  }
  assert.ok(!workflows.targets.test.dependsOn.includes("policy-tests"));
  assert.equal(workflows.targets["policy-tests"], undefined);
  // Every policy journey sits under github-workflow-policy's root, which github-workflows'
  // own inputs leave out, so editing one alone does not select the CD journeys.
  assert.ok(workflows.namedInputs.default.includes("!{projectRoot}/policy/**/*"));
  for (const file of policyJourneyFiles) assert.ok(file.startsWith(".github/policy/"), file);
  const files = [...policyJourneyFiles, ...cdJourneyFiles];
  const actual = ["tests", "policy/tests"].flatMap((directory) =>
    readdirSync(new URL(`../${directory}/`, import.meta.url))
      .filter((file) => file.endsWith(".test.mjs"))
      .map((file) => `.github/${directory}/${file}`),
  );
  assert.deepEqual(files.sort(), actual.sort());
});

// Selection by real Nx, which is what the split is for: a crate edit selects the CD journeys
// and not the policy journeys, a policy journey edit the reverse, and a workflow edit both.
test("a crate edit selects the CD journeys without the policy journeys", () => {
  const require = createRequire(import.meta.url);
  const nx = resolve(
    dirname(require.resolve("nx/package.json")),
    require("nx/package.json").bin.nx,
  );
  const cwd = fileURLToPath(new URL("../../", import.meta.url));
  const selected = (file) =>
    JSON.parse(
      execFileSync(
        process.execPath,
        [nx, "show", "projects", "--affected", "--files", file, "--json"],
        {
          cwd,
          encoding: "utf8",
        },
      ),
    );
  for (const [file, workflows, policy] of [
    ["crates/onetaskgraph-core/src/lib.rs", true, false],
    ["sdks/python/project.json", true, false],
    [".github/policy/tests/policy-routing.test.mjs", false, true],
    [".github/policy/budgets.yaml", false, true],
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
