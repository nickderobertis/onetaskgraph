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
} from "../../test-inputs.mjs";
import { graphReadSet } from "../../workflow-graph.mjs";

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
  for (const file of policyJourneyFiles) assert.ok(file.startsWith(".github/policy/"), file);
  const actual = ["tests", "policy/tests"].flatMap((directory) =>
    readdirSync(new URL(`../../${directory}/`, import.meta.url))
      .filter((file) => file.endsWith(".test.mjs"))
      .map((file) => `.github/${directory}/${file}`),
  );
  assert.deepEqual([...policyJourneyFiles, ...cdJourneyFiles].sort(), actual.sort());
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
