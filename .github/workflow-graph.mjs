import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import { parse } from "yaml";
import { isMacosRunner } from "./telemetry.ts";
import { data, Evaluator, Lexer, Parser } from "./actions-expressions.mjs";

export const root = new URL("../", import.meta.url);
export const read = (path) => readFileSync(new URL(path, root), "utf8");
function record(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
function validatedJob(job, path) {
  const scalarMap = (value) =>
    record(value) &&
    Object.values(value).every((entry) => ["string", "boolean", "number"].includes(typeof entry));
  if (
    !record(job) ||
    (!job.uses && !job["runs-on"]) ||
    (job.uses !== undefined && typeof job.uses !== "string") ||
    (job.name !== undefined && typeof job.name !== "string") ||
    (job.needs !== undefined &&
      !(
        typeof job.needs === "string" ||
        (Array.isArray(job.needs) && job.needs.every((id) => typeof id === "string" && id))
      )) ||
    (job.if !== undefined && typeof job.if !== "string") ||
    ["outputs", "with", "env"].some(
      (field) => job[field] !== undefined && !scalarMap(job[field]),
    ) ||
    (job.strategy !== undefined && !record(job.strategy)) ||
    (job.steps !== undefined &&
      (!Array.isArray(job.steps) ||
        job.steps.some(
          (step) =>
            !record(step) ||
            ["run", "uses", "id"].some(
              (field) => step[field] !== undefined && typeof step[field] !== "string",
            ) ||
            (step.env !== undefined && !scalarMap(step.env)),
        )))
  )
    throw Error(`Unresolved workflow job: ${path}`);
}
function reusableInputs(child, supplied) {
  const callable =
    typeof child.on === "string"
      ? child.on === "workflow_call"
        ? {}
        : undefined
      : Array.isArray(child.on)
        ? child.on.includes("workflow_call")
          ? {}
          : undefined
        : child.on?.workflow_call;
  if (callable === undefined || (callable !== null && !record(callable)))
    throw Error("Unresolved reusable workflow_call declaration");
  const declarations = callable?.inputs === undefined ? {} : callable.inputs;
  if (!record(declarations) || !record(supplied)) throw Error("Unresolved reusable inputs");
  for (const key of Object.keys(supplied))
    if (!(key in declarations)) throw Error(`Unknown reusable input: ${key}`);
  const result = {};
  for (const [key, definition] of Object.entries(declarations)) {
    if (
      !record(definition) ||
      !["string", "boolean", "number"].includes(definition.type) ||
      (definition.required !== undefined && typeof definition.required !== "boolean")
    )
      throw Error(`Unresolved reusable input declaration: ${key}`);
    const value = supplied[key] ?? definition.default;
    if (value === undefined) {
      if (definition.required) throw Error(`Missing required reusable input: ${key}`);
      result[key] = { string: "", boolean: false, number: 0 }[definition.type];
    } else {
      if (typeof value !== definition.type) throw Error(`Invalid reusable input type: ${key}`);
      result[key] = value;
    }
  }
  return result;
}
function resolvedCron(value) {
  if (typeof value !== "string") return false;
  const fields = value.trim().split(/\s+/);
  const bounds = [
    [0, 59],
    [0, 23],
    [1, 31],
    [1, 12],
    [0, 6],
  ];
  // These workflows use literal fields or wildcards; other cron forms remain explicit errors.
  return (
    fields.length === bounds.length &&
    fields.every(
      (field, index) =>
        field === "*" ||
        (/^\d+$/.test(field) &&
          Number(field) >= bounds[index][0] &&
          Number(field) <= bounds[index][1]),
    )
  );
}
function validatedWorkflow(value, path) {
  if (
    !value ||
    typeof value !== "object" ||
    Array.isArray(value) ||
    (value.name !== undefined && (typeof value.name !== "string" || !value.name.trim())) ||
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
  if (
    (Array.isArray(value.on) &&
      (!value.on.length || value.on.some((event) => typeof event !== "string" || !event))) ||
    (record(value.on) && !Object.keys(value.on).length)
  )
    throw Error(`Unresolved workflow trigger list: ${path}`);
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
      spec?.inputs !== undefined &&
      (!record(spec.inputs) ||
        Object.values(spec.inputs).some(
          (definition) =>
            !record(definition) ||
            !["string", "boolean", "number"].includes(definition.type) ||
            (definition.required !== undefined && typeof definition.required !== "boolean") ||
            (definition.default !== undefined && typeof definition.default !== definition.type),
        ))
    )
      throw Error(`Unresolved workflow inputs: ${path}/${event}`);
    if (
      !known.has(event) ||
      (event === "schedule"
        ? !Array.isArray(spec) ||
          !spec.length ||
          spec.some((row) => !row || !resolvedCron(row.cron))
        : spec !== null && (typeof spec !== "object" || Array.isArray(spec)))
    )
      throw Error(`Unresolved workflow trigger: ${path}/${event}`);
  }
  for (const [id, job] of Object.entries(value.jobs)) validatedJob(job, `${path}/${id}`);
  return value;
}
export const workflows = () =>
  Object.fromEntries(
    readdirSync(new URL(".github/workflows/", root))
      .filter((p) => /\.ya?ml$/.test(p))
      .map((p) => [p, validatedWorkflow(parse(read(`.github/workflows/${p}`)), p)]),
  );
export function validateEventInventory(inventory, definitions = workflows()) {
  if (!Array.isArray(inventory) || !inventory.length)
    throw Error("Invalid event inventory: expected nonempty array");
  const identifiers = new Set();
  for (const entry of inventory) {
    if (
      !entry ||
      typeof entry !== "object" ||
      Array.isArray(entry) ||
      typeof entry.id !== "string" ||
      !/^[a-z][a-z0-9-]*$/.test(entry.id) ||
      identifiers.has(entry.id) ||
      typeof entry.event !== "string" ||
      !/^[a-z][a-z_]*$/.test(entry.event) ||
      ["action", "branch", "tag"].some(
        (field) =>
          entry[field] !== undefined && (typeof entry[field] !== "string" || !entry[field]),
      ) ||
      (entry.fork !== undefined && typeof entry.fork !== "boolean") ||
      (entry.inputs !== undefined &&
        (!entry.inputs ||
          typeof entry.inputs !== "object" ||
          Array.isArray(entry.inputs) ||
          Object.entries(entry.inputs).some(
            ([key, value]) => !/^[a-z][a-z0-9_-]*$/.test(key) || typeof value !== "boolean",
          )))
    )
      throw Error("Invalid event inventory entry");
    identifiers.add(entry.id);
  }
  // Closed PRs are the merge phases driven separately by the lifecycle journey.
  const cases = [...inventory, { event: "pull_request", action: "closed" }];
  for (const [name, workflow] of Object.entries(definitions)) {
    const declarations =
      typeof workflow.on === "string"
        ? { [workflow.on]: null }
        : Array.isArray(workflow.on)
          ? Object.fromEntries(workflow.on.map((event) => [event, null]))
          : workflow.on;
    for (const [event, options] of Object.entries(declarations)) {
      if (event === "workflow_call") continue;
      const matching = cases.filter((entry) => entry.event === event);
      if (!matching.length) throw Error(`Unmeasured workflow event: ${name}: ${event}`);
      const actions =
        options?.types || (event === "pull_request" ? ["opened", "synchronize", "reopened"] : []);
      for (const action of actions)
        if (!matching.some((entry) => entry.action === action))
          throw Error(`Unmeasured workflow action: ${name}: ${event}/${action}`);
      for (const [filter, field] of [
        ["branches", "branch"],
        ["tags", "tag"],
      ])
        for (const selection of options?.[filter] ?? [])
          if (
            !matching.some((entry) =>
              field === "branch"
                ? !entry.tag && (entry.branch || "main") === selection
                : entry.tag === selection,
            )
          )
            throw Error(`Unmeasured workflow ${filter}: ${name}: ${selection}`);
      // graph() fans out every measured upstream run through workflow_run.
      // Refuse names outside that graph rather than assuming no downstream run.
      for (const upstream of options?.workflows ?? [])
        if (!Object.values(definitions).some((definition) => definition.name === upstream))
          throw Error(`Unmeasured workflow upstream: ${name}: ${upstream}`);
      if (options?.inputs !== undefined && !record(options.inputs))
        throw Error(`Unmeasured workflow input declaration: ${name}`);
      for (const [input, definition] of Object.entries(options?.inputs ?? {})) {
        if (!record(definition) || definition.type !== "boolean")
          throw Error(`Unmeasured workflow input type: ${name}: ${input}`);
        for (const value of [false, true])
          if (!matching.some((entry) => entry.inputs?.[input] === value))
            throw Error(`Unmeasured workflow input: ${name}: ${input}=${value}`);
      }
    }
  }
}
export function digest() {
  const files = [
    "release-targets.toml",
    "screencomp.toml",
    "scripts/product_versions.py",
    "scripts/release_products.py",
    ...readdirSync(new URL(".github/", root))
      .filter((p) => /\.(mjs|py|json|lock|ts)$/.test(p))
      .map((p) => `.github/${p}`),
    ...readdirSync(new URL(".github/workflows/", root)).map((p) => `.github/workflows/${p}`),
    ...readdirSync(new URL(".github/reusable/", root)).map((p) => `.github/reusable/${p}`),
    ...readdirSync(new URL(".github/tests/", root), { recursive: true, withFileTypes: true })
      .filter((entry) => entry.isFile())
      .map((entry) =>
        relative(fileURLToPath(root), join(entry.parentPath, entry.name)).replaceAll("\\", "/"),
      ),
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
    if (/^\s*\$\{\{(?:(?!\$\{\{)[\s\S])*\}\}\s*$/.test(value)) return evaluate(value, context);
    return value.replace(/\$\{\{(.*?)\}\}/g, (_, expr) => String(evaluate(expr, context)));
  }
  if (Array.isArray(value)) return value.map((v) => resolve(v, context));
  if (value && typeof value === "object")
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, resolve(v, context)]));
  return value;
}
export function matrices(matrix, context) {
  if (matrix === undefined || matrix === null) return [{}];
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
  if (!Object.keys(resolved).length) throw Error("Unresolved matrix: empty declaration");
  const axes = Object.entries(resolved).filter(([k]) => !["include", "exclude"].includes(k));
  let rows = axes.length ? [{}] : [];
  for (const [key, values] of axes) {
    if (!Array.isArray(values) || !values.length) throw Error(`Unresolved matrix axis ${key}`);
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
  if (scenario.event === "push") {
    if (scenario.tag) {
      if (spec?.tags ? !spec.tags.includes(scenario.tag) : spec?.branches) return false;
    } else if (spec?.branches ? !spec.branches.includes(scenario.branch || "main") : spec?.tags)
      return false;
  } else if (spec?.branches && !spec.branches.includes(scenario.branch || "main")) return false;
  return true;
}
function reusable(call) {
  if (call.startsWith("./")) {
    if (!/^\.\/\.github\/workflows\/[A-Za-z0-9_.-]+\.ya?ml$/.test(call))
      throw Error(`Unresolved local reusable call: ${call}`);
    return validatedWorkflow(parse(read(call.slice(2))), call);
  }
  const lock = JSON.parse(read(".github/reusable-lock.json"))[call];
  if (!lock) throw Error(`Unresolved reusable workflow: ${call}`);
  if (
    typeof lock.path !== "string" ||
    !/^\.github\/reusable\/[A-Za-z0-9_.-]+\.ya?ml$/.test(lock.path) ||
    typeof lock.sha256 !== "string" ||
    !/^[a-f0-9]{64}$/.test(lock.sha256) ||
    typeof lock.upstreamSha256 !== "string" ||
    !/^[a-f0-9]{64}$/.test(lock.upstreamSha256)
  )
    throw Error(`Unresolved reusable lock entry: ${call}`);
  const source = read(lock.path);
  if (createHash("sha256").update(source).digest("hex") !== lock.sha256)
    throw Error(`Reusable source digest mismatch: ${call}`);
  if (
    createHash("sha256")
      .update(source.slice(source.indexOf("\n") + 1))
      .digest("hex") !== lock.upstreamSha256
  )
    throw Error(`Reusable upstream source drift: ${call}`);
  return validatedWorkflow(parse(source), call);
}
const archMatrices = new Map();
// rust-cache-warm.yml's release-merge decision, made by running that job's own step over the
// revisions a push scenario names ({ cwd, before, after }): a push naming none is undecidable
// here rather than assumed either way.
const releaseDecisions = new Map();
function releaseMerge(job, context, scenario) {
  const step = job.steps?.find((candidate) => candidate.id === "decide");
  if (!step?.run) throw Error("Unresolved release-merge decision command");
  const revisions = scenario.revisions;
  if (!revisions?.cwd || !revisions.before || !revisions.after)
    throw Error("Unresolved release-merge revisions: a push scenario names none");
  const stepEnvironment = resolve(step.env || {}, context);
  const key = JSON.stringify([step.run, stepEnvironment, revisions.cwd]);
  if (!releaseDecisions.has(key)) {
    const scratch = mkdtempSync(
      join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir(), "workflow-release-merge-"),
    );
    try {
      const output = join(scratch, "output");
      // As GitHub runs a `shell: bash` step: a step that exits non-zero fails its job, which
      // then has no outputs — a run outcome rather than an unresolved workflow.
      const ran = spawnSync("bash", ["--noprofile", "--norc", "-eo", "pipefail", "-c", step.run], {
        cwd: revisions.cwd,
        env: { ...process.env, ...stepEnvironment, GITHUB_OUTPUT: output },
        stdio: ["ignore", "ignore", "ignore"],
      });
      if (ran.error) throw ran.error;
      if (ran.status !== 0) releaseDecisions.set(key, null);
      else {
        const answer = readFileSync(output, "utf8")
          .split(/\r?\n/)
          .find((line) => line.startsWith("version-only="))
          ?.slice("version-only=".length);
        if (!["true", "false"].includes(answer)) throw Error("Unresolved release-merge output");
        releaseDecisions.set(key, answer);
      }
    } finally {
      rmSync(scratch, { recursive: true, force: true });
    }
  }
  const answer = releaseDecisions.get(key);
  if (answer === null) return { result: "failure", outputs: {} };
  return {
    result: "success",
    outputs: resolve(job.outputs, {
      ...context,
      steps: { decide: { outputs: { "version-only": answer } } },
    }),
  };
}

let configuredArches;
export function jobs(workflow, scenario, policy, prefix = "", inputs = {}, ancestry = []) {
  const rows = [];
  const needs = {};
  const pending = new Map(Object.entries(workflow.jobs));
  for (const [id, job] of pending) validatedJob(job, `${prefix}${id}`);
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
            before: scenario.revisions?.before || "",
            pull_request: { head: { repo: { fork: scenario.fork } } },
          },
          ref: scenario.tag
            ? `refs/tags/${scenario.tag}`
            : `refs/heads/${scenario.branch || "main"}`,
          sha: scenario.revisions?.after || "",
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
      if (!enabled) continue;
      if (id === "macos-lanes") {
        context.steps = {
          select: {
            outputs: { macosRequired: String(policy.macosRequired), reason: policy.reason },
          },
        };
        needs[id].outputs = resolve(job.outputs, context);
      }
      if (id === "release-merge") needs[id] = releaseMerge(job, context, scenario);
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
        const step = job.steps.find((candidate) => candidate.id === "read");
        if (!step?.run) throw Error("Unresolved reusable arches command");
        const stepEnvironment = resolve(step.env || {}, {
          ...context,
          inputs: { ...context.inputs, arches: JSON.stringify(arches) },
        });
        const key = JSON.stringify([step.run, stepEnvironment]);
        if (!archMatrices.has(key)) {
          const scratch = mkdtempSync(
            join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir(), "workflow-arches-"),
          );
          try {
            const output = join(scratch, "output");
            execFileSync("bash", ["-c", step.run], {
              cwd: root,
              env: {
                ...process.env,
                ...stepEnvironment,
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
      for (const matrix of matrices(job.strategy?.matrix, context)) {
        const rowContext = { ...context, matrix };
        if (job.uses) {
          if (ancestry.includes(job.uses)) throw Error("Recursive reusable workflow");
          const child = reusable(job.uses);
          const supplied = reusableInputs(child, resolve(job.with || {}, rowContext));
          rows.push(
            ...jobs(child, scenario, policy, `${prefix}${id} / `, supplied, [
              ...ancestry,
              job.uses,
            ]),
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
        let declaredTargets = "";
        if (job.env?.TARGETS !== undefined) {
          const pairs = resolve(job.env.TARGETS, rowContext);
          if (
            typeof pairs !== "string" ||
            !pairs.trim() ||
            pairs
              .trim()
              .split(/\s+/)
              .some((pair) => !/^[a-z0-9_]+(?:-[a-z0-9_]+)+=[a-z0-9]+-[a-z0-9]+$/.test(pair))
          )
            throw Error(`Unresolved release TARGETS: ${id}`);
          declaredTargets = pairs
            .trim()
            .split(/\s+/)
            .map((pair) => pair.split("=")[0])
            .join(",");
        }
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
          target: matrix.target || matrix.arch || declaredTargets,
          macos: isMacosRunner(labels),
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
