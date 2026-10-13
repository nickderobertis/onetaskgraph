import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parse } from "yaml";
import { data, Evaluator, Lexer, Parser } from "./actions-expressions.mjs";

export const root = new URL("../", import.meta.url);
export const read = (path) => readFileSync(new URL(path, root), "utf8");
function validatedWorkflow(value, path) {
  if (
    !value ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    !value.jobs ||
    typeof value.jobs !== "object" ||
    Array.isArray(value.jobs) ||
    !Object.keys(value.jobs).length ||
    !(
      typeof value.on === "string" ||
      Array.isArray(value.on) ||
      (value.on && typeof value.on === "object")
    )
  )
    throw Error(`Unresolved workflow declaration: ${path}`);
  const events =
    typeof value.on === "string"
      ? { [value.on]: null }
      : Array.isArray(value.on)
        ? Object.fromEntries(value.on.map((event) => [event, null]))
        : value.on;
  const known = new Set([
    "push",
    "pull_request",
    "release",
    "workflow_run",
    "repository_dispatch",
    "schedule",
    "workflow_dispatch",
    "workflow_call",
  ]);
  for (const [event, spec] of Object.entries(events)) {
    if (
      !known.has(event) ||
      (event === "schedule"
        ? !Array.isArray(spec) ||
          !spec.length ||
          spec.some((row) => !row || typeof row.cron !== "string")
        : spec !== null && (typeof spec !== "object" || Array.isArray(spec)))
    )
      throw Error(`Unresolved workflow trigger: ${path}/${event}`);
  }
  for (const [id, job] of Object.entries(value.jobs)) {
    if (!job || typeof job !== "object" || Array.isArray(job) || (!job.uses && !job["runs-on"]))
      throw Error(`Unresolved workflow job: ${path}/${id}`);
  }
  return value;
}
export const workflows = () =>
  Object.fromEntries(
    readdirSync(new URL(".github/workflows/", root))
      .filter((p) => /\.ya?ml$/.test(p))
      .map((p) => [p, validatedWorkflow(parse(read(`.github/workflows/${p}`)), p)]),
  );
export function digest() {
  const files = [
    "release-targets.toml",
    "screencomp.toml",
    "scripts/product_versions.py",
    ...readdirSync(new URL(".github/", root))
      .filter((p) => /\.(mjs|py|json|lock|ts)$/.test(p))
      .map((p) => `.github/${p}`),
    ...readdirSync(new URL(".github/workflows/", root)).map((p) => `.github/workflows/${p}`),
    ...readdirSync(new URL(".github/reusable/", root)).map((p) => `.github/reusable/${p}`),
    ...readdirSync(new URL(".github/tests/", root)).map((p) => `.github/tests/${p}`),
  ];
  return createHash("sha256")
    .update(
      files
        .sort()
        .map((p) => `${p}\0${read(p)}`)
        .join("\0"),
    )
    .digest("hex");
}
export function evaluate(expression, context) {
  const text = expression.replace(/^\s*\$\{\{\s*|\s*\}\}\s*$/g, "");
  const outcomes = Object.values(context.needs || {}).map((n) => n.result);
  const funcs = new Map(
    Object.entries({
      always: true,
      cancelled: outcomes.includes("cancelled"),
      success: outcomes.every((r) => r === "success"),
      failure: outcomes.includes("failure"),
    }).map(([name, value]) => [
      name,
      { name, minArgs: 0, maxArgs: 0, call: () => new data.BooleanData(value) },
    ]),
  );
  const ast = new Parser(new Lexer(text).lex().tokens, Object.keys(context), [
    ...funcs.values(),
  ]).parse();
  const result = new Evaluator(
    ast,
    JSON.parse(JSON.stringify(context), data.reviver),
    funcs,
  ).evaluate();
  return JSON.parse(JSON.stringify(result, data.replacer));
}
export function resolve(value, context) {
  if (typeof value === "string") {
    if (/^\s*\$\{\{[\s\S]*\}\}\s*$/.test(value)) return evaluate(value, context);
    return value.replace(/\$\{\{(.*?)\}\}/g, (_, expr) => String(evaluate(expr, context)));
  }
  if (Array.isArray(value)) return value.map((v) => resolve(v, context));
  if (value && typeof value === "object")
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, resolve(v, context)]));
  return value;
}
export function matrices(matrix, context) {
  if (!matrix) return [{}];
  const resolved = resolve(matrix, context);
  if (!resolved || typeof resolved !== "object" || Array.isArray(resolved))
    throw Error("Unresolved matrix");
  for (const key of ["include", "exclude"])
    if (
      resolved[key] !== undefined &&
      (!Array.isArray(resolved[key]) ||
        resolved[key].some((row) => !row || typeof row !== "object" || Array.isArray(row)))
    )
      throw Error(`Unresolved matrix ${key} entries`);
  const axes = Object.entries(resolved).filter(([k]) => !["include", "exclude"].includes(k));
  let rows = axes.length ? [{}] : [];
  for (const [key, values] of axes) {
    if (!Array.isArray(values)) throw Error(`Unresolved matrix axis ${key}`);
    rows = rows.flatMap((row) => values.map((v) => ({ ...row, [key]: v })));
  }
  rows = rows.filter(
    (row) =>
      !(resolved.exclude || []).some((ex) => Object.entries(ex).every(([k, v]) => row[k] === v)),
  );
  const originals = rows.map((row) => ({ ...row }));
  for (const inc of resolved.include || []) {
    let merged = false;
    originals.forEach((row, i) => {
      if (Object.entries(inc).every(([k, v]) => !(k in row) || row[k] === v)) {
        Object.assign(rows[i], inc);
        merged = true;
      }
    });
    if (!merged) rows.push(inc);
  }
  return rows;
}
export function triggered(workflow, scenario) {
  validatedWorkflow(workflow, "event routing");
  const on = workflow.on;
  const spec =
    typeof on === "string"
      ? on === scenario.event
        ? {}
        : undefined
      : Array.isArray(on)
        ? on.includes(scenario.event)
          ? {}
          : undefined
        : on?.[scenario.event];
  if (spec === undefined) return false;
  for (const key of ["types", "branches", "tags", "workflows"])
    if (
      spec?.[key] !== undefined &&
      (!Array.isArray(spec[key]) || spec[key].some((value) => typeof value !== "string" || !value))
    )
      throw Error(`Unresolved event filter values: ${key}`);
  const supported = new Set(["types", "branches", "tags", "workflows", "inputs"]);
  if (spec && !Array.isArray(spec) && typeof spec === "object")
    for (const key of Object.keys(spec))
      if (!supported.has(key)) throw Error(`Unresolved event filter: ${key}`);
  for (const key of ["branches", "tags", "workflows"])
    if (spec?.[key]?.some((pattern) => /[*!?[\\]/.test(pattern)))
      throw Error(`Unresolved Actions glob filter: ${key}`);
  if (spec?.workflows && !spec.workflows.includes(scenario.workflow)) return false;
  if (
    scenario.event === "pull_request" &&
    !spec?.types &&
    !["opened", "synchronize", "reopened"].includes(scenario.action)
  )
    return false;
  if (spec?.types && !spec.types.includes(scenario.action)) return false;
  if (spec?.branches && !spec.branches.includes(scenario.branch || "main")) return false;
  if (spec?.tags && !spec.tags.includes(scenario.tag)) return false;
  if (scenario.tag && spec?.branches && !spec.tags) return false;
  return true;
}
function reusable(call) {
  if (call.startsWith("./")) {
    if (!call.startsWith("./.github/workflows/") || call.split("/").includes(".."))
      throw Error(`Unresolved local reusable call: ${call}`);
    return validatedWorkflow(parse(read(call.slice(2))), call);
  }
  const lock = JSON.parse(read(".github/reusable-lock.json"))[call];
  if (!lock) throw Error(`Unresolved reusable workflow: ${call}`);
  if (
    typeof lock.path !== "string" ||
    !lock.path.startsWith(".github/reusable/") ||
    lock.path.split("/").some((part) => part === ".." || !part) ||
    typeof lock.sha256 !== "string" ||
    !/^[a-f0-9]{64}$/.test(lock.sha256)
  )
    throw Error(`Unresolved reusable lock entry: ${call}`);
  const source = read(lock.path);
  if (createHash("sha256").update(source).digest("hex") !== lock.sha256)
    throw Error(`Reusable source digest mismatch: ${call}`);
  if (
    lock.upstreamSha256 &&
    createHash("sha256")
      .update(source.slice(source.indexOf("\n") + 1))
      .digest("hex") !== lock.upstreamSha256
  )
    throw Error(`Reusable upstream source drift: ${call}`);
  return validatedWorkflow(parse(source), call);
}
const archMatrices = new Map();
let configuredArches;
export function jobs(workflow, scenario, policy, prefix = "", inputs = {}, ancestry = []) {
  const rows = [];
  const needs = {};
  const pending = new Map(Object.entries(workflow.jobs));
  while (pending.size) {
    let advanced = false;
    for (const [id, job] of pending) {
      const dependencies = typeof job.needs === "string" ? [job.needs] : job.needs || [];
      if (dependencies.some((d) => !(d in needs))) continue;
      advanced = true;
      pending.delete(id);
      const context = {
        github: {
          event_name: scenario.event,
          event: {
            action: scenario.action,
            pull_request: { head: { repo: { fork: scenario.fork } } },
          },
          ref: scenario.tag
            ? `refs/tags/${scenario.tag}`
            : `refs/heads/${scenario.branch || "main"}`,
        },
        needs: Object.fromEntries(dependencies.map((d) => [d, needs[d]])),
        inputs: { ...(scenario.inputs || {}), ...inputs },
        vars: scenario.vars || {},
        matrix: {},
        steps: {},
        runner: {},
      };
      const implicitSuccess = dependencies.every((d) => needs[d].result === "success");
      const statusOverride = job.if && /\b(always|cancelled|failure|success)\(/.test(job.if);
      const enabled =
        (implicitSuccess || statusOverride) && (!job.if || Boolean(evaluate(job.if, context)));
      needs[id] = { result: enabled ? "success" : "skipped", outputs: {} };
      if (id === "macos-lanes") {
        context.steps = {
          select: {
            outputs: { macosRequired: String(policy.macosRequired), reason: policy.reason },
          },
        };
        needs[id].outputs = resolve(job.outputs, context);
      }
      if (id === "arches") {
        configuredArches ||= JSON.parse(
          execFileSync(
            "python3",
            [
              "-c",
              'import tomllib,json; print(json.dumps(tomllib.load(open("screencomp.toml","rb"))["capture"]["arches"]))',
            ],
            { cwd: root, encoding: "utf8" },
          ),
        );
        const arches = configuredArches;
        if (
          !Array.isArray(arches) ||
          !arches.length ||
          arches.some((arch) => !["x86_64", "arm64", "aarch64"].includes(arch))
        )
          throw Error("Unresolved capture architecture list");
        const key = JSON.stringify([arches, inputs]);
        if (!archMatrices.has(key)) {
          const scratch = mkdtempSync(
            join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir(), "workflow-arches-"),
          );
          try {
            const output = join(scratch, "output");
            const step = job.steps.find((s) => s.id === "read");
            if (!step?.run) throw Error("Unresolved reusable arches command");
            execFileSync("bash", ["-c", step.run], {
              cwd: root,
              env: {
                ...process.env,
                ARCHES_OVERRIDE: JSON.stringify(arches),
                PROJECTS_INPUT: inputs.projects || "",
                DEFAULT_TITLE: inputs["gallery-title"] || "",
                GITHUB_OUTPUT: output,
              },
            });
            const matrix = readFileSync(output, "utf8")
              .split("\n")
              .find((line) => line.startsWith("matrix="))
              ?.slice(7);
            if (!matrix) throw Error("Unresolved reusable arches output");
            archMatrices.set(key, matrix);
          } finally {
            rmSync(scratch, { recursive: true, force: true });
          }
        }
        needs[id].outputs.matrix = archMatrices.get(key);
      }
      if (!enabled) continue;
      for (const matrix of matrices(job.strategy?.matrix, context)) {
        const rowContext = { ...context, matrix };
        if (job.uses) {
          if (ancestry.includes(job.uses)) throw Error("Recursive reusable workflow");
          const child = reusable(job.uses);
          const defaults = Object.fromEntries(
            Object.entries(child.on?.workflow_call?.inputs || {}).map(([k, v]) => [k, v.default]),
          );
          rows.push(
            ...jobs(
              child,
              scenario,
              policy,
              `${prefix}${id} / `,
              { ...defaults, ...resolve(job.with || {}, rowContext) },
              [...ancestry, job.uses],
            ),
          );
          continue;
        }
        const runner = resolve(job["runs-on"], rowContext);
        const labels =
          typeof runner === "string" ? [runner] : Array.isArray(runner) ? runner : runner?.labels;
        if (
          !Array.isArray(labels) ||
          !labels.length ||
          labels.some((x) => typeof x !== "string" || !x || x.includes("${{"))
        )
          throw Error(`Unresolved runner: ${prefix}${id}`);
        if (
          labels.some((x) => x === "self-hosted") &&
          !labels.some((x) => /macos|linux|windows/i.test(x))
        )
          throw Error(`Unknown self-hosted OS: ${prefix}${id}`);
        if (
          !labels.some((label) =>
            /^(?:ubuntu-[a-z0-9.-]+|windows-[a-z0-9.-]+|macos-[a-z0-9.-]+)$/i.test(label),
          )
        )
          throw Error(`Unresolved runner OS: ${prefix}${id}`);
        rows.push({
          job: `${prefix}${id}`,
          name: `${prefix}${resolve(job.name || id, rowContext)}${
            !job.name && Object.keys(matrix).length
              ? ` (${Object.values(matrix)
                  .filter((v) => v !== "")
                  .join(", ")})`
              : ""
          }`,
          runner: labels,
          target:
            matrix.target ||
            matrix.arch ||
            (job.env?.TARGETS
              ? resolve(job.env.TARGETS, rowContext)
                  .split(" ")
                  .map((pair) => pair.split("=")[0])
                  .join(",")
              : ""),
          macos: labels.some((x) => /macos/i.test(x)),
          matrix,
        });
      }
    }
    if (!advanced) throw Error(`Unresolved job needs: ${[...pending.keys()].join(",")}`);
  }
  return rows;
}
export function graph(scenario, policy, declared = workflows()) {
  const runs = [];
  const queue = [{ scenario, chain: [] }];
  while (queue.length) {
    const current = queue.shift();
    for (const [workflow, wf] of Object.entries(declared)) {
      if (!triggered(wf, current.scenario)) continue;
      if (current.chain.includes(workflow))
        throw Error(`Unresolved workflow_run cycle: ${workflow}`);
      runs.push({
        workflow,
        event: current.scenario.event,
        action: current.scenario.action,
        ...(current.scenario.workflow ? { triggeredBy: current.scenario.workflow } : {}),
        rows: jobs(wf, current.scenario, policy),
      });
      queue.push({
        scenario: {
          ...current.scenario,
          event: "workflow_run",
          action: "completed",
          workflow: wf.name,
        },
        chain: [...current.chain, workflow],
      });
    }
  }
  return runs;
}
