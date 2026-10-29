// `github-approval-calls-points`: the modelled GraphQL points of one complete design approval
// `tests/e2e/approval.rs` recorded in the crate's test run — its review task created through
// `task project-human`, the four existing store calls, and the review resolved — the larger of
// the unscreened and the screened sequence, handed to onebudgetspec with each call and the split
// between existing calls and the review's projection as its detail. Reads saved telemetry only.
import { report } from "@onebudgetspec/sdk";
import { recorded, run, sum } from "./telemetry.mjs";

const BUDGET = "github-approval-calls-points";
const WORKLOAD = "plan approval store calls";
const CALLS = [
  "task project-human (review open)",
  "project show",
  "task list --project",
  "document list --project",
  "document metadata set --expect",
  "task project-human (review completed)",
];

// One mode's figures, refused unless its calls are the approval's, in order, adding up.
function mode(telemetry, name) {
  const spent = telemetry[name];
  if (!spent || !Array.isArray(spent.calls)) throw Error(`the recording has no ${name} sequence`);
  if (JSON.stringify(spent.calls.map((call) => call.call)) !== JSON.stringify(CALLS))
    throw Error(`the ${name} sequence does not name the approval's calls in order`);
  const projection = spent.calls.filter((call) => call.call.startsWith("task project-human"));
  if (
    sum(spent.calls, "points") !== spent.points ||
    sum(projection, "points") !== spent.projection ||
    spent.existing + spent.projection !== spent.points
  )
    throw Error(`the ${name} sequence's points do not add up`);
  return spent;
}

const reported = run(BUDGET, () => {
  const telemetry = recorded(BUDGET, WORKLOAD);
  if (telemetry.board !== 2000 || telemetry.tasks !== 8 || telemetry.documents !== 1)
    throw Error("the recording is not of a plan of 8 tasks and 1 document on a 2000-item board");
  const unscreened = mode(telemetry, "unscreened");
  const screened = mode(telemetry, "screened");
  const value = Math.max(unscreened.points, screened.points);
  if (value !== telemetry.value) throw Error("the recording's figure is not its larger sequence");
  const split = ({ points, existing, projection, calls }) => ({
    points,
    existing,
    projection,
    calls,
  });
  return {
    value,
    detail: JSON.stringify({
      unit: "primary modelled GraphQL points",
      unscreened: split(unscreened),
      screened: split(screened),
    }),
  };
});
if (reported) report(reported.value, reported.detail);
