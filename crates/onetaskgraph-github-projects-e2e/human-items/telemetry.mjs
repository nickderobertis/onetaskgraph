// The saved telemetry the crate's own `test` target recorded, as both human-items reporters read
// it: nothing here measures, and nothing here judges a figure — onebudgetspec does.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");

// Where `onetaskgraph-github-projects-e2e:test` writes its telemetry — the directory its
// `outputs` declare — or wherever GITHUB_ASSET_TELEMETRY_DIR moved it, as the journeys honour.
export function telemetryDirectory() {
  return (
    process.env.GITHUB_ASSET_TELEMETRY_DIR ??
    join(root, "target", "debug", "telemetry", "onetaskgraph-github-projects-e2e")
  );
}

// The recording of `budget`, refused unless it is that budget's, of `workload`.
export function recorded(budget, workload) {
  const path = join(telemetryDirectory(), `${budget}.json`);
  let telemetry;
  try {
    telemetry = JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    throw Error(`no telemetry at ${path}: ${error.message}`);
  }
  if (telemetry.budget !== budget || telemetry.workload !== workload)
    throw Error(`${path} is not the ${workload} recording of ${budget}`);
  return telemetry;
}

// The sum of `field` over `rows`, each a non-negative integer.
export function sum(rows, field) {
  return rows.reduce((total, row) => {
    if (!Number.isInteger(row[field]) || row[field] < 0)
      throw Error(`a row's ${field} is not a count: ${JSON.stringify(row)}`);
    return total + row[field];
  }, 0);
}

// Run `reporter`, printing what it handed over or why it refused.
export function run(budget, reporter) {
  try {
    const { value, detail } = reporter();
    console.log(JSON.stringify({ budget, value }));
    return { value, detail };
  } catch (error) {
    console.error(
      `${budget}: ${error.message}; next: run onetaskgraph-github-projects-e2e:test, then the reporter again`,
    );
    process.exitCode = 1;
    return null;
  }
}
