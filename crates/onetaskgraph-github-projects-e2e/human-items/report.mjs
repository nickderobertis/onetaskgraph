// `human-task-projection-points`: the modelled GraphQL points per item of the projection
// lifecycle `tests/e2e/human_cost.rs` recorded in the crate's test run, handed to onebudgetspec
// with each operation's and phase's requests, points, aliased mutation fields and secondary
// points as its detail. Reads saved telemetry only; re-runs nothing.
import { report } from "@onebudgetspec/sdk";
import { recorded, run, sum } from "./telemetry.mjs";

const BUDGET = "human-task-projection-points";
const WORKLOAD = "human task projection lifecycle";
const COUNTS = ["requests", "points", "mutation_fields", "secondary_points"];

const reported = run(BUDGET, () => {
  const telemetry = recorded(BUDGET, WORKLOAD);
  const { tasks, managers, board_items, screened, totals, phases, operations } = telemetry;
  if (tasks !== 100 || managers !== 10 || board_items !== 2000 || screened !== true)
    throw Error("the recording is not of 100 tasks from 10 managers on a screened 2000-item board");
  for (const field of COUNTS) {
    if (sum(phases, field) !== totals[field] || sum(operations, field) !== totals[field])
      throw Error(`the recording's ${field} do not add up to its total`);
  }
  for (const phase of ["create", "terminal", "create_recovery", "cold_terminal_retry"]) {
    if (!phases.some((row) => row.phase === phase))
      throw Error(`the recording has no ${phase} phase`);
  }
  return {
    value: totals.points / tasks,
    detail: JSON.stringify({
      unit: "primary modelled GraphQL points per item",
      tasks,
      managers,
      board_items,
      terminals: telemetry.terminals,
      recoveries: telemetry.recoveries,
      secondary_model: telemetry.secondary_model,
      totals,
      phases,
      operations,
    }),
  };
});
if (reported) report(reported.value, reported.detail);
