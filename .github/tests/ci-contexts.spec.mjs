import {
  cdJourneyReadSet,
  policyJourneyReadSet,
  cdJourneyFiles,
  policyJourneyFiles,
} from "../test-inputs.mjs";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { test } from "node:test";
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
  return [
    ...outside
      .filter((path) => !inputs.includes(`{workspaceRoot}/${path}`))
      .map((path) => `${target} is not keyed on ${path}`),
    ...inputs
      .filter((entry) => String(entry).startsWith("{workspaceRoot}/"))
      .filter((input) => !outside.includes(input.slice("{workspaceRoot}/".length)))
      .map((input) => `${target} is keyed on ${input}, which its tests do not read`),
  ];
}

test("each target is keyed on the combined read set of the tests it runs", () => {
  const project = JSON.parse(readFileSync(new URL("../project.json", import.meta.url), "utf8"));
  for (const target of ["ci-contexts", "policy-tests", "cd-journeys"])
    assert.deepEqual(inputProblems(project, target), []);
});

test("an undeclared read in either journey target is refused", () => {
  const graph = graphReadSet().filter((path) => !path.startsWith(".github/"));
  for (const [target, reads] of [
    ["policy-tests", policyJourneyReadSet],
    ["cd-journeys", cdJourneyReadSet],
  ]) {
    for (const path of new Set([...graph, ...reads])) {
      const project = JSON.parse(readFileSync(new URL("../project.json", import.meta.url), "utf8"));
      for (const [name, inputs] of Object.entries(project.namedInputs))
        project.namedInputs[name] = inputs.filter((input) => input !== `{workspaceRoot}/${path}`);
      assert.ok(
        inputProblems(project, target).includes(`${target} is not keyed on ${path}`),
        `${target}: ${path}`,
      );
    }
  }
});

test("the aggregate runs every journey exactly once through its owning target", () => {
  const project = JSON.parse(readFileSync(new URL("../project.json", import.meta.url), "utf8"));
  assert.equal(project.targets.test.executor, "nx:noop");
  for (const [target, files] of [
    ["policy-tests", policyJourneyFiles],
    ["cd-journeys", cdJourneyFiles],
  ]) {
    assert.ok(project.targets.test.dependsOn.includes(target));
    assert.equal(project.targets[target].options.command, `node --test ${files.join(" ")}`);
  }
  const files = [...policyJourneyFiles, ...cdJourneyFiles];
  const actual = readdirSync(new URL("./", import.meta.url))
    .filter((file) => file.endsWith(".test.mjs"))
    .map((file) => `.github/tests/${file}`);
  assert.deepEqual(files.sort(), actual.sort());
});
