import { readFileSync } from "node:fs";
import { report } from "@onebudgetspec/sdk";
import { validateTelemetry } from "./telemetry.ts";
import { digest } from "./workflow-graph.mjs";

try {
  const args = process.argv.slice(2);
  if (
    args.length !== 4 ||
    args[0] !== "--metric" ||
    args[2] !== "--telemetry" ||
    !["jobs-per-run", "runs-per-change"].includes(args[1])
  )
    throw Error("usage: --metric jobs-per-run|runs-per-change --telemetry JSON");
  const telemetry = JSON.parse(readFileSync(args[3], "utf8"));
  validateTelemetry(telemetry);
  if (
    telemetry.schemaVersion !== 1 ||
    telemetry.sourceDigest !== digest() ||
    !telemetry.scenarios?.length ||
    !telemetry.lifecycles?.length
  )
    throw Error("Missing or stale macOS telemetry; run github-workflows:test");
  const runs = telemetry.scenarios.flatMap((s) =>
    s.runs.map((r) => ({ ...r, scenario: s.id, diff: s.diff, fork: s.fork, optional: s.vars })),
  );
  const value =
    args[1] === "jobs-per-run"
      ? Math.max(
          ...runs.map((r) => r.macJobs),
          ...telemetry.lifecycles.flatMap((lifecycle) => lifecycle.runs.map((run) => run.macJobs)),
        )
      : Math.max(...telemetry.lifecycles.map((l) => l.macRuns));
  const detail = JSON.stringify({
    sourceDigest: telemetry.sourceDigest,
    runs: runs
      .filter((r) => r.macJobs)
      .map((r) => ({
        workflow: r.workflow,
        event: r.event,
        scenario: r.scenario,
        diff: r.diff,
        fork: r.fork,
        optional: r.optional,
        jobs: r.rows.filter((j) => j.macos),
      })),
    lifecycles: telemetry.lifecycles.map((l) => ({
      diff: l.diff,
      fork: l.fork,
      optional: l.optional,
      runs: l.runs
        .filter((r) => r.macJobs)
        .map((r) => ({
          workflow: r.workflow,
          event: r.event,
          phase: r.phase,
          jobs: r.rows.filter((j) => j.macos),
        })),
    })),
  });
  report(value, detail);
  console.log(JSON.stringify({ metric: args[1], value }));
} catch (error) {
  console.error(`${error.message}; next: run github-workflows:test and retry the reporter`);
  process.exitCode = 1;
}
