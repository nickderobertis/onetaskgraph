import "./ambient.ts";
import { afterAll, expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { OnetaskgraphClient, OnetaskgraphExecutionError } from "../src/index.ts";

// `sources sanitize` through the SDK: its report comes back on exits 1 and 3, not as an error,
// and a sweep that could not run is still the error it is. The binary is the real one; the
// policy is two small commands — one listing the store's own replacements as the terms, one
// rewriting them and refusing any identifier carrying one, as onevcs's sanitise command does.

const binary = resolve(import.meta.dir, "../../../target/debug/onetaskgraph");
const TERM = "emberline";

// Validate the subprocess input against the pinned policy contract before using it.
const policySchema = JSON.parse(
  readFileSync(
    resolve(
      import.meta.dir,
      "../../../crates/onetaskgraph-core/src/engine/boundary/onevcs-boundary-schema-v4.json",
    ),
    "utf8",
  ),
);
const ajvPath = createRequire(import.meta.url).resolve("ajv/dist/2020.js");
const policyInput = (schema: unknown) => `
const Ajv = require(${JSON.stringify(ajvPath)});
const validate = new Ajv({ strict: false }).compile(${JSON.stringify(schema)});
let input;
try { input = JSON.parse(require("node:fs").readFileSync(0, "utf8")); }
catch { process.exit(2); }
if (!validate(input)) process.exit(2);
`;

const TERMS =
  policyInput(policySchema.terms.input) +
  `
const rules = [...new Map((input.replacements ?? []).map((entry) => [entry.term, entry])).values()];
console.log(JSON.stringify({ terms: rules
  .map((entry) => ({ term: entry.term, mode: "whole-word", case_sensitive: false,
                     action: entry.reject === true ? "reject" : "replace",
                     ...("with" in entry ? { with: entry.with } : {}) })) }));
`;

const SANITIZE =
  policyInput(policySchema.sanitize.input) +
  `
const rules = [...new Map((input.replacements ?? []).map((entry) => [entry.term, entry])).values()];
const terms = rules.filter((entry) => "with" in entry);
let replaced = 0;
const rewrite = (value) => terms.reduce((text, entry) => {
  replaced += text.split(entry.term).length - 1;
  return text.split(entry.term).join(entry.with);
}, value);
const answer = {};
for (const surface of ["text", "paths", "metadata"]) answer[surface] = (input[surface] ?? []).map(rewrite);
const unfixed = [];
for (const [key, surface] of [["text", "text"], ["paths", "path"], ["metadata", "metadata"]]) {
  for (const [index, value] of answer[key].entries()) {
    if (rules.some((entry) => entry.reject === true && value.includes(entry.term)))
      unfixed.push({ surface, index, fixed: false });
  }
  for (const [index, value] of (input.fixed?.[key] ?? []).entries()) {
    if (rules.some((entry) => value.includes(entry.term))) unfixed.push({ surface, index, fixed: true });
  }
}
const locators = input.locators ?? [];
const carrying = locators.flatMap((locator, index) =>
  rules.some((entry) => locator.value.includes(entry.term)) ? [index] : []);
let ordinal = 0;
answer.locators = locators.map((locator, index) => carrying.includes(index)
  ? { redacted: true, kind: locator.kind, ordinal: ++ordinal } : locator.value);
answer.replaced = replaced;
unfixed.push(...carrying.map((index) => ({ surface: "locators", index, fixed: true })));
if (unfixed.length > 0) {
  Object.assign(answer, { verdict: "refuse", surface: unfixed[0].surface, unfixed });
  console.log(JSON.stringify(answer));
  process.exit(1);
}
Object.assign(answer, { verdict: "pass", unfixed: [] });
console.log(JSON.stringify(answer));
`;

const roots: string[] = [];

test("policy subprocesses refuse malformed wire inputs before consuming them", () => {
  for (const command of [TERMS, SANITIZE]) {
    for (const input of [
      [],
      { replacements: [{}] },
      { text: [7] },
      { locators: [{ kind: "item", value: 7 }] },
    ]) {
      const result = spawnSync(process.execPath, ["-e", command], {
        input: JSON.stringify(input),
        encoding: "utf8",
      });
      expect(result.status).toBe(2);
      expect(result.stdout).toBe("");
    }
  }
});

afterAll(() => {
  for (const root of roots) rmSync(root, { recursive: true, force: true });
});

// A store over `sources` whose policy replaces TERM, listing its terms only when `terms` is set.
function store(sources: Record<string, unknown>, terms = true): [OnetaskgraphClient, string] {
  const root = mkdtempSync(resolve(tmpdir(), "onetaskgraph-sweep-"));
  roots.push(root);
  writeFileSync(
    resolve(root, "onetaskgraph.yaml"),
    JSON.stringify({
      write_policy: {
        check_command: [process.execPath, "-e", "process.exit(2)"],
        sanitize_command: [process.execPath, "-e", SANITIZE],
        ...(terms ? { terms_command: [process.execPath, "-e", TERMS] } : {}),
        replacements: [{ term: TERM, with: "ember-x" }],
        // A sweep does not read this; the write preview's journey here drives the guard it opts into.
        sanitize_stored_on_write: true,
      },
      sources,
    }),
  );
  return [new OnetaskgraphClient({ binaryPath: binary, cwd: root }), root];
}

test("per-call replacement and rejection overrides support a sanitization preview", async () => {
  const [, root] = store({});
  const folder = resolve(root, "plans");
  mkdirSync(resolve(folder, "tasks"), { recursive: true });
  const file = resolve(folder, "tasks/t1.md");
  const before = `---\ntitle: Retry\nstatus: todo\n---\nRetry as ${TERM} does.\n`;
  writeFileSync(file, before);
  const [client] = store({
    plans: { plugin: "local-md", visibility: "public", config: { root: folder } },
  });
  const preview = await client.taskPrioritySet("plans:t1", "high", {
    sanitizeDryRun: true,
    replace: [`${TERM}=flag=x`],
  });
  expect("would_pass" in preview).toBe(true);
  if (!("would_pass" in preview)) throw new Error("expected a sanitization preview");
  expect(preview.would_pass).toBe(true);
  expect(preview.sanitization?.dry_run).toBe(true);
  expect(readFileSync(file, "utf8")).toBe(before);
  await expect(
    client.taskPrioritySet("plans:t1", "high", { reject: [TERM] }),
  ).rejects.toBeInstanceOf(OnetaskgraphExecutionError);
  expect(readFileSync(file, "utf8")).toBe(before);
  const written = await client.taskPrioritySet("plans:t1", "high", { replace: [`${TERM}=flag=x`] });
  expect(written.priority).toBe("high");
  expect(written.sanitization?.side_effect.length).toBeGreaterThan(0);
  expect(readFileSync(file, "utf8")).toContain("flag=x");
  expect(readFileSync(file, "utf8")).not.toContain(TERM);
  writeFileSync(file, before);
  const swept = await client.sourcesSanitize("plans", {
    replace: [`${TERM}=sweep-x`],
    dryRun: true,
  });
  expect(swept.items[0]?.outcome).toBe("would-rewrite");
  expect(readFileSync(file, "utf8")).toBe(before);
  await client.sourcesSanitize("plans", { replace: [`${TERM}=sweep-x`] });
  expect(readFileSync(file, "utf8")).toContain("sweep-x");
});

test("a search sweep answers its report on exit 3", async () => {
  const uv = Bun.which("uv");
  if (uv === null) throw new Error("uv is needed to run the metered Python SDK peer");
  const pythonProject = resolve(import.meta.dir, "../../python");
  const [, root] = store({});
  const held = resolve(root, "tracker.json");
  const task = (id: string, content: string) => ({
    id,
    title: id,
    content,
    status: { category: "todo", name: "Todo" },
    labels: [],
    project: null,
    url: null,
    location: null,
    created_at: null,
    updated_at: null,
  });
  writeFileSync(
    held,
    JSON.stringify({ requests: 0, tasks: [task("t1", `As ${TERM} does.`), task("t2", "Plain.")] }),
  );
  const peer = resolve(import.meta.dir, "../../python/tests/sweep_source.py");
  const [client] = store({
    tracker: {
      plugin: "subprocess",
      visibility: "public",
      config: {
        command: resolve(uv),
        // The plugin environment is cleared, including CI's UV_NO_SYNC. Bootstrap already
        // prepared this SDK environment; a peer must not rebuild the workspace CLI here.
        args: ["run", "--frozen", "--no-sync", "--project", pythonProject, "python", peer, held],
      },
    },
  });
  const report = await client.sourcesSanitize("tracker");
  expect(report.coverage).toEqual({
    complete: false,
    uncovered: ["metadata", "comments", "normalized-spellings"],
  });
  expect(report.items.map((item) => [item.id, item.outcome])).toEqual([
    ["tracker:t1", "rewritten"],
  ]);
  // `complete` reads the source whole, covering every class a search cannot.
  const whole = await client.sourcesSanitize("tracker", { complete: true, dryRun: true });
  expect(whole.coverage).toEqual({ complete: true, uncovered: [] });
  expect(whole.dry_run).toBe(true);
  const after = JSON.parse(readFileSync(held, "utf8")).tasks;
  expect(after.find((entry: { id: string }) => entry.id === "t1").content).toBe("As ember-x does.");
}, 30_000);

test("a sweep leaving an identifier unfixable answers its report on exit 1", async () => {
  const [client, root] = store({
    site: { plugin: "local-md", config: { root: "site" }, visibility: "public" },
  });
  mkdirSync(resolve(root, "site", "tasks"), { recursive: true });
  writeFileSync(
    resolve(root, "site", "tasks", `${TERM}-note.md`),
    "---\ntitle: Note\nstatus: todo\n---\nPlain.\n",
  );
  const report = await client.sourcesSanitize("site");
  expect(report.coverage.complete).toBe(true);
  expect(report.items).toHaveLength(1);
  expect(report.items[0]?.outcome).toBe("unfixable");
  expect(report.items[0]?.id).toEqual({ redacted: true, kind: "item", ordinal: 1 });
});

test("a sweep that cannot run still rejects", async () => {
  const [client, root] = store(
    { site: { plugin: "local-md", config: { root: "site" }, visibility: "public" } },
    false,
  );
  mkdirSync(resolve(root, "site"));
  const refused = client.sourcesSanitize("site");
  await expect(refused).rejects.toBeInstanceOf(OnetaskgraphExecutionError);
  await expect(refused).rejects.toThrow("terms_command");
});

test("sourcesSanitize refuses arguments it cannot send, before the binary is started", () => {
  const client = new OnetaskgraphClient({ binaryPath: "/nonexistent/onetaskgraph", cwd: tmpdir() });
  // Widened on purpose: these are the arguments a JavaScript caller, unchecked by TypeScript,
  // can still pass, and the method has to refuse them at run time.
  const sanitize = client.sourcesSanitize.bind(client) as (...args: unknown[]) => unknown;
  expect(() => sanitize(7)).toThrow("source is not a string");
  expect(() => sanitize("site", null)).toThrow("options is not a plain object");
  expect(() => sanitize("site", ["--complete"])).toThrow("options is not a plain object");
  expect(() => sanitize("site", { complete: "yes" })).toThrow("complete is not a boolean");
  expect(() => sanitize("site", { dryRun: 1 })).toThrow("dryRun is not a boolean");
  expect(() => sanitize("site", { termScope: ["invented/project"] })).toThrow(
    "scope is derived from the source",
  );
  expect(() => sanitize("site", { termScope: [] })).toThrow("scope is derived from the source");
});
