import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { read, workflows } from "../workflow-graph.mjs";

// The service owns the queue. Reconcile its actual declarations instead of simulating
// a second scheduler whose behavior could disagree with Actions.
test("ordinary, daily and explicit initialization retain three pending CI writers", () => {
  const definitions = workflows();
  const ci = definitions["ci.yml"];
  const creation = definitions["linear-documents.yml"];
  assert.equal(ci.concurrency, undefined, "outer cancellation must not cancel pending checks");
  const ordinary = ci.jobs.check.concurrency;
  assert.match(ordinary.group, /'linear-live-documents'/);
  const daily = creation.jobs.documents.concurrency;
  assert.equal(daily.group, "linear-live-documents");
  // Three simultaneous pending writers must all reach the service's 100-waiter queue;
  // the default single pending setting would silently replace two required checks.
  for (const declaration of [ordinary, daily, daily]) {
    assert.equal(declaration["cancel-in-progress"], false);
    assert.equal(declaration.queue, "max");
  }
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
  for (const step of definitions["ci.yml"].jobs.check.steps.filter(
    (step) => step.env?.LINEAR_API_KEY,
  )) {
    assert.equal(step.env.ONETASKGRAPH_LINEAR_DOCUMENT_MODE, "ordinary");
    assert.match(step.run, /^just (check|gate)-generated$/);
  }
});

test("surface edits select document policy without publication journeys", () => {
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
    assert.ok(
      !projects.includes("github-workflows"),
      `${file} must not select publication journeys`,
    );
  }
});
