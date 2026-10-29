import assert from "node:assert/strict";
import { test } from "node:test";
import { workflows } from "../workflow-graph.mjs";

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
  assert.equal(commands.filter((command) => command.includes("--test live")).length, 1);
  assert.equal(
    commands.filter((command) =>
      command.includes("linear_assets::live_disposable_issue_and_document_assets_copy_and_recopy"),
    ).length,
    1,
  );
  for (const step of definitions["ci.yml"].jobs.check.steps.filter(
    (step) => step.env?.LINEAR_API_KEY,
  )) {
    assert.equal(step.env.ONETASKGRAPH_LINEAR_DOCUMENT_MODE, "ordinary");
    assert.match(step.run, /^just (check|gate)-generated$/);
  }
});
