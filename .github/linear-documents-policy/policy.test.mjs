import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { read, workflows } from "../workflow-graph.mjs";

// No lock serialises the live suites: every session works only on its own run's board copy
// and leased Linear slot set, so runs execute them side by side. A job-level or
// workflow-level concurrency group on anything that runs a live suite would bring back the
// queue a pull request once waited hours in, so this refuses one wherever it appears.
const LIVE_CREDENTIALS = ["LINEAR_API_KEY", "GH_PROJECTS_TOKEN"];

/** The jobs of `definitions` that run a live suite: a credentialed step that runs `just`. */
export function liveSuiteJobs(definitions) {
  const found = [];
  for (const [file, workflow] of Object.entries(definitions)) {
    for (const [name, job] of Object.entries(workflow.jobs ?? {})) {
      const jobCredentialed = LIVE_CREDENTIALS.some((key) => job.env?.[key] !== undefined);
      const runsSuite = (job.steps ?? []).some(
        (step) =>
          typeof step.run === "string" &&
          /(^|\s)just\s/.test(step.run) &&
          (jobCredentialed || LIVE_CREDENTIALS.some((key) => step.env?.[key] !== undefined)),
      );
      if (runsSuite) found.push({ file, name, workflow, job });
    }
  }
  return found;
}

/** Every serialisation of a live suite `definitions` declares, as one line each. */
export function liveSerialisations(definitions, sources) {
  const problems = [];
  for (const { file, name, workflow, job } of liveSuiteJobs(definitions)) {
    if (job.concurrency !== undefined) {
      problems.push(`${file} job ${name} runs a live suite under a concurrency group`);
    }
    if (workflow.concurrency !== undefined) {
      problems.push(`${file} runs a live suite (job ${name}) under a workflow concurrency group`);
    }
  }
  for (const [file, text] of Object.entries(sources)) {
    if (text.includes("linear-live-documents")) {
      problems.push(`${file} names the retired linear-live-documents lock`);
    }
  }
  return problems;
}

const workflowSources = () =>
  Object.fromEntries(
    Object.keys(workflows()).map((file) => [file, read(`.github/workflows/${file}`)]),
  );

test("no workflow serialises a live suite", () => {
  const definitions = workflows();
  const jobs = liveSuiteJobs(definitions).map(({ file, name }) => `${file}:${name}`);
  // The scan has to see every lane that opens a live session, or a pass proves nothing.
  for (const expected of ["ci.yml:check", "ci.yml:test-linux", "linear-documents.yml:documents"]) {
    assert.ok(jobs.includes(expected), `${expected} runs a live suite and was not found`);
  }
  assert.deepEqual(liveSerialisations(definitions, workflowSources()), []);
});

test("a reintroduced live lock is refused, at job and at workflow level", () => {
  const definitions = structuredClone(workflows());
  definitions["ci.yml"].jobs["test-linux"].concurrency = { group: "any-shared-group" };
  definitions["linear-documents.yml"].concurrency = { group: "documents" };
  const sources = { "ci.yml": "concurrency:\n  group: linear-live-documents\n" };
  assert.deepEqual(liveSerialisations(definitions, sources), [
    "ci.yml job test-linux runs a live suite under a concurrency group",
    "linear-documents.yml runs a live suite (job documents) under a workflow concurrency group",
    "ci.yml names the retired linear-live-documents lock",
  ]);
});

test("only the daily schedule creates fresh documents and manual dispatch initializes", () => {
  const definitions = workflows();
  const workflow = definitions["linear-documents.yml"];
  assert.deepEqual(Object.keys(workflow.on), ["schedule", "workflow_dispatch"]);
  assert.deepEqual(workflow.on.schedule, [{ cron: "43 6 * * *" }]);
  assert.equal(workflow.on.workflow_dispatch, null);
  const job = workflow.jobs.documents;
  assert.equal(
    job.env.ONETASKGRAPH_LINEAR_DOCUMENT_MODE,
    "$" + "{{ github.event_name == 'schedule' && 'fresh' || 'initialize' }}",
  );
  assert.equal(job.env.ONETASKGRAPH_LIVE_REQUIRED, "1");
  assert.equal(job.env.LINEAR_WRITE_TEAM, "$" + "{{ vars.LINEAR_WRITE_TEAM }}");
  const commands = job.steps.filter((step) => step.run).map((step) => step.run);
  assert.deepEqual(commands, ["just linear-document-creation"]);
  assert.ok(job.steps.some((step) => step.uses === "extractions/setup-just@v4"));
  assert.ok(
    job.steps.some(
      (step) =>
        step.uses === "oven-sh/setup-bun@v2" && step.with["bun-version-file"] === "package.json",
    ),
  );
  assert.match(
    read("justfile"),
    /linear-document-creation:\n {4}@\{\{nx\}\} run-many -t live-documents --projects onetaskgraph-linear,onetaskgraph-linear-e2e --parallel=1/,
  );
  const commandsByLane = {
    "onetaskgraph-linear":
      "cargo test --locked -p onetaskgraph-linear --test live -- real_linear_applies_every_declared_capability_and_leaves_no_residue --exact --nocapture",
    "onetaskgraph-linear-e2e":
      "cargo test --locked -p onetaskgraph-linear-e2e --test e2e -- linear_assets::live_disposable_issue_and_document_assets_copy_and_recopy --exact --nocapture",
  };
  for (const [lane, command] of Object.entries(commandsByLane)) {
    const project = JSON.parse(read(`crates/${lane}/project.json`));
    const target = project.targets["live-documents"];
    assert.equal(target.options.command, command);
    assert.equal(target.cache, false);
    assert.equal(target.parallelism, false);
    assert.ok(!project.targets.check.dependsOn.includes("live-documents"));
    if (lane.endsWith("-e2e")) assert.deepEqual(target.dependsOn, ["onetaskgraph:build"]);
  }
  const credentialed = (job) =>
    definitions["ci.yml"].jobs[job].steps.filter((step) => step.env?.LINEAR_API_KEY);
  const ordinarySteps = [...credentialed("check"), ...credentialed("test-linux")];
  assert.ok(credentialed("test-linux").length > 0, "test-linux carries a pull request's session");
  for (const step of ordinarySteps) {
    assert.equal(step.env.ONETASKGRAPH_LINEAR_DOCUMENT_MODE, "ordinary");
    assert.match(step.run, /^just ((check|gate)-generated|ci-test-linux)$/);
  }
});

// The operator's two setup recipes reach the lanes only through the variable each lane reads
// and the entry point that reads it, so their spellings are held to the lanes' own.
test("the pool and template recipes set the variables their lanes read", () => {
  const constant = (path, name) =>
    read(path).match(new RegExp(`pub const ${name}: &str = "([^"]+)";`))[1];
  const pool = constant(
    "crates/onetaskgraph-linear/tests/document_lifecycle/mod.rs",
    "POOL_VARIABLE",
  );
  const template = constant(
    "crates/onetaskgraph-github-projects/tests/journey/run_board.rs",
    "TEMPLATE_VARIABLE",
  );
  const recipe = (name) => read("justfile").split(`\n${name} action:\n`)[1].split("\n")[0];
  const poolRecipe = recipe("linear-document-pool");
  assert.ok(poolRecipe.includes(`${pool}={{action}}`), poolRecipe);
  assert.ok(
    poolRecipe.includes("-t live-documents --projects onetaskgraph-linear,onetaskgraph-linear-e2e"),
    poolRecipe,
  );
  const templateRecipe = recipe("github-live-template");
  assert.ok(templateRecipe.includes(`${template}={{action}}`), templateRecipe);
  assert.ok(templateRecipe.includes("GITHUB_LIVE_SELECTION=run"), templateRecipe);
  assert.ok(
    templateRecipe.includes(
      "--test live -- real_projects_v2_contract_writes_and_leaves_no_residue --exact",
    ),
    templateRecipe,
  );
  for (const [lane, journey] of [
    ["onetaskgraph-linear", "real_linear_applies_every_declared_capability_and_leaves_no_residue"],
    [
      "onetaskgraph-linear-e2e",
      "linear_assets::live_disposable_issue_and_document_assets_copy_and_recopy",
    ],
  ]) {
    const project = JSON.parse(read(`crates/${lane}/project.json`));
    assert.ok(project.targets["live-documents"].options.command.includes(journey), lane);
  }
});

// These surfaces select the policy. Whether they also select github-workflows is that
// project's own read set to say: its CD journeys build and package every crate and run the
// justfile's release recipes, so .github/test-inputs.mjs names both, reconciled by ci-contexts.
test("surface edits select document policy", () => {
  const require = createRequire(import.meta.url);
  const manifest = require.resolve("nx/package.json");
  const nx = resolve(dirname(manifest), require("nx/package.json").bin.nx);
  const cwd = fileURLToPath(new URL("../../", import.meta.url));
  for (const file of [
    "justfile",
    "crates/onetaskgraph-linear/project.json",
    "crates/onetaskgraph-linear-e2e/project.json",
  ]) {
    const projects = JSON.parse(
      execFileSync(
        process.execPath,
        [nx, "show", "projects", "--affected", "--files", file, "--json"],
        { cwd, encoding: "utf8" },
      ),
    );
    assert.ok(projects.includes("linear-documents-policy"), `${file} must select its policy`);
  }
  const policyOnly = JSON.parse(
    execFileSync(
      process.execPath,
      [
        nx,
        "show",
        "projects",
        "--affected",
        "--files",
        ".github/linear-documents-policy/policy.test.mjs",
        "--json",
      ],
      { cwd, encoding: "utf8" },
    ),
  );
  assert.ok(policyOnly.includes("linear-documents-policy"));
  assert.ok(
    !policyOnly.includes("github-workflows"),
    "the policy alone must not select publication journeys",
  );
});
