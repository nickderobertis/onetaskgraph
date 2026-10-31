// onetaskgraph-gate-time's command: reports the wall clock of the `just gate` run that
// scripts/gate-time.sh recorded, through the onebudgetspec SDK. It measures nothing itself — a
// development machine's gate is the measurement, and this reads it back from the one file the
// wrapper writes beside the repository (gitignored). A missing, malformed or failed record is an
// error, never a figure: onebudgetspec then exits 3 and the gate fails.
import { readFileSync } from "node:fs";
import { report } from "@onebudgetspec/sdk";

const RECORD = new URL("../.gate-time.json", import.meta.url);

try {
  let text;
  try {
    text = readFileSync(RECORD, "utf8");
  } catch {
    throw Error("no gate record at .gate-time.json: run `just gate`, which writes it");
  }
  let record;
  try {
    record = JSON.parse(text);
  } catch {
    throw Error(
      ".gate-time.json is not JSON: delete it and run `just gate`, which writes it afresh",
    );
  }
  const started = Date.parse(record?.started_at);
  const ended = Date.parse(record?.ended_at);
  if (
    record?.schema_version !== 1 ||
    !Number.isFinite(started) ||
    !Number.isFinite(ended) ||
    ended < started ||
    !Number.isInteger(record.exit)
  )
    throw Error(
      ".gate-time.json is not a gate record scripts/gate-time.sh wrote: delete it and run `just gate`",
    );
  if (record.exit !== 0)
    throw Error(
      `the recorded gate exited ${record.exit}, and a failed gate has no wall clock to report: fix what it reported and run \`just gate\` again`,
    );
  const seconds = Math.round((ended - started) / 1000);
  report(
    seconds,
    `just gate ran ${seconds}s, from ${record.started_at} to ${record.ended_at} (invocation ${record.invocation})`,
  );
} catch (error) {
  console.error(`gate-time-budget: ${error.message}`);
  process.exitCode = 1;
}
