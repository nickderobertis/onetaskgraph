/** Runtime validation narrows the recorded graph before the SDK reporter consumes it. */
export interface JobRow {
  job: string;
  name: string;
  runner: string[];
  target: string;
  macos: boolean;
  matrix: Record<string, unknown>;
}
export interface WorkflowRun {
  workflow: string;
  event: string;
  rows: JobRow[];
  macJobs: number;
  phase?: string;
}
export interface Scenario {
  id: string;
  diff: string;
  fork: boolean;
  vars: Record<string, unknown>;
  runs: WorkflowRun[];
}
export interface Lifecycle {
  diff: string;
  fork: boolean;
  optional: number;
  runs: WorkflowRun[];
  macRuns: number;
}
export interface Telemetry {
  schemaVersion: 1;
  sourceDigest: string;
  scenarios: Scenario[];
  lifecycles: Lifecycle[];
}
function object(value: unknown): asserts value is Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw Error("Invalid telemetry object");
}
function array(value: unknown): asserts value is unknown[] {
  if (!Array.isArray(value)) throw Error("Invalid telemetry array");
}
function string(value: unknown): asserts value is string {
  if (typeof value !== "string" || !value.length) throw Error("Invalid telemetry string");
}
function count(value: unknown): asserts value is number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0)
    throw Error("Invalid telemetry measurement");
}
function job(value: unknown): asserts value is JobRow {
  object(value);
  string(value.job);
  string(value.name);
  array(value.runner);
  object(value.matrix);
  if (!value.runner.length || typeof value.target !== "string" || typeof value.macos !== "boolean")
    throw Error("Invalid telemetry runner/job");
  const labels = value.runner.map((label) => {
    string(label);
    return label;
  });
  const macos = labels.some((label) => /^macos-[a-z0-9.-]+$/i.test(label));
  if (
    value.macos !== macos ||
    !labels.some((label) => /^(?:ubuntu|windows|macos)-[a-z0-9.-]+$/i.test(label))
  )
    throw Error("Telemetry runner classification disagrees with labels");
}
function run(value: unknown): asserts value is WorkflowRun {
  object(value);
  string(value.workflow);
  string(value.event);
  count(value.macJobs);
  array(value.rows);
  if (value.phase !== undefined) string(value.phase);
  const rows = value.rows.map((row) => {
    job(row);
    return row;
  });
  if (value.macJobs !== rows.filter((row) => row.macos).length)
    throw Error("Telemetry job total disagrees with rows");
}
function scenario(value: unknown): asserts value is Scenario {
  object(value);
  string(value.id);
  string(value.diff);
  object(value.vars);
  array(value.runs);
  if (typeof value.fork !== "boolean") throw Error("Invalid telemetry ownership");
  for (const row of value.runs) run(row);
}
export function validateTelemetry(value: unknown): asserts value is Telemetry {
  object(value);
  string(value.sourceDigest);
  array(value.scenarios);
  array(value.lifecycles);
  if (
    value.schemaVersion !== 1 ||
    !/^[a-f0-9]{64}$/.test(value.sourceDigest) ||
    !value.scenarios.length ||
    !value.lifecycles.length
  )
    throw Error("Invalid telemetry envelope");
  const scenarios = value.scenarios.map((row) => {
    scenario(row);
    return row;
  });
  if (!scenarios.some((row) => row.runs.length))
    throw Error("Telemetry has no workflow measurements");
  for (const lifecycle of value.lifecycles) {
    object(lifecycle);
    string(lifecycle.diff);
    count(lifecycle.optional);
    count(lifecycle.macRuns);
    array(lifecycle.runs);
    if (typeof lifecycle.fork !== "boolean") throw Error("Invalid telemetry ownership");
    if (!lifecycle.runs.length) throw Error("Telemetry has no lifecycle measurements");
    const runs = lifecycle.runs.map((row) => {
      run(row);
      return row;
    });
    if (lifecycle.macRuns !== runs.filter((row) => row.macJobs > 0).length)
      throw Error("Telemetry lifecycle total disagrees with runs");
  }
}
