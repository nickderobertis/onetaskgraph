import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { graph, readSet } from "../workflow-graph.mjs";
import { definitions, refusals } from "./workflow-definitions.mjs";

// Today's required contexts, as the run graph resolves a pull request's runs. The graph reads
// files outside .github (readSet() names them), so this is the github-workflows project's
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

// Each target that runs the graph (this spec, and the *.test.mjs suites) is keyed on exactly the
// files outside .github that the graph reads, so a change to one re-runs it and selects it.
test("every target that reads the run graph is keyed on each file outside .github it reads", () => {
  const project = JSON.parse(readFileSync(new URL("../project.json", import.meta.url), "utf8"));
  const expand = (inputs) =>
    inputs.flatMap((input) =>
      typeof input === "string" && project.namedInputs?.[input]
        ? expand(project.namedInputs[input])
        : [input],
    );
  const outside = readSet().filter((path) => !path.startsWith(".github/"));
  assert.ok(
    outside.length > 0,
    "the run graph reads nothing outside .github, so this proves nothing",
  );
  for (const target of ["ci-contexts", "test"]) {
    const inputs = expand(project.targets[target].inputs ?? ["default"]);
    for (const path of outside)
      assert.ok(inputs.includes(`{workspaceRoot}/${path}`), `${target} is not keyed on ${path}`);
    for (const input of inputs.filter((entry) => String(entry).startsWith("{workspaceRoot}/")))
      assert.ok(
        outside.includes(input.slice("{workspaceRoot}/".length)),
        `${target} is keyed on ${input}, which the run graph does not read`,
      );
  }
});
