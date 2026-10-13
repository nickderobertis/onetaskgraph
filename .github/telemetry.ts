/** The recorded graph is an input boundary; reporting must not trust its numeric rows. */
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
function run(value: unknown): asserts value is WorkflowRun {
  object(value);
  string(value.workflow);
  string(value.event);
  count(value.macJobs);
  array(value.rows);
  for (const row of value.rows) {
    object(row);
    string(row.job);
    string(row.name);
    array(row.runner);
    if (!row.runner.length) throw Error("Missing telemetry runner");
    for (const label of row.runner) string(label);
    if (typeof row.target !== "string" || typeof row.macos !== "boolean")
      throw Error("Invalid telemetry job");
    object(row.matrix);
    const macos = row.runner.some((label) => /^macos-[a-z0-9.-]+$/i.test(label as string));
    if (
      row.macos !== macos ||
      !row.runner.some((label) => /^(?:ubuntu|windows|macos)-[a-z0-9.-]+$/i.test(label as string))
    )
      throw Error("Telemetry runner classification disagrees with labels");
  }
  if (value.macJobs !== value.rows.filter((row) => (row as JobRow).macos).length)
    throw Error("Telemetry job total disagrees with rows");
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
  for (const scenario of value.scenarios) {
    object(scenario);
    string(scenario.id);
    string(scenario.diff);
    object(scenario.vars);
    array(scenario.runs);
    if (typeof scenario.fork !== "boolean") throw Error("Invalid telemetry ownership");
    for (const row of scenario.runs) run(row);
  }
  if (!value.scenarios.some((scenario) => (scenario as Scenario).runs.length))
    throw Error("Telemetry has no workflow measurements");
  for (const lifecycle of value.lifecycles) {
    object(lifecycle);
    string(lifecycle.diff);
    count(lifecycle.optional);
    count(lifecycle.macRuns);
    array(lifecycle.runs);
    if (typeof lifecycle.fork !== "boolean") throw Error("Invalid telemetry ownership");
    if (!lifecycle.runs.length) throw Error("Telemetry has no lifecycle measurements");
    for (const row of lifecycle.runs) run(row);
    if (
      lifecycle.macRuns !== lifecycle.runs.filter((row) => (row as WorkflowRun).macJobs > 0).length
    )
      throw Error("Telemetry lifecycle total disagrees with runs");
  }
}
