import "./ambient.ts";
import { afterAll, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { OnetaskgraphClient, OnetaskgraphExecutionError } from "../src/index.ts";

// `sources sanitize` through the SDK: its report comes back on exits 1 and 3, not as an error,
// and a sweep that could not run is still the error it is. The binary is the real one; the
// policy is two small commands — one listing the store's own replacements as the terms, one
// rewriting them and refusing any identifier carrying one, as onevcs's sanitise command does.

const binary = resolve(import.meta.dir, "../../../target/debug/onetaskgraph");
const TERM = "emberline";

const TERMS = `
const input = JSON.parse(require("node:fs").readFileSync(0, "utf8"));
console.log(JSON.stringify({ terms: (input.replacements ?? []).filter((entry) => "with" in entry)
  .map((entry) => ({ term: entry.term, mode: "whole-word", case_sensitive: false,
                     action: "replace", with: entry.with })) }));
`;

const SANITIZE = `
const input = JSON.parse(require("node:fs").readFileSync(0, "utf8"));
const terms = (input.replacements ?? []).filter((entry) => "with" in entry);
let replaced = 0;
const rewrite = (value) => terms.reduce((text, entry) => {
  replaced += text.split(entry.term).length - 1;
  return text.split(entry.term).join(entry.with);
}, value);
const answer = {};
for (const surface of ["text", "paths", "metadata"]) answer[surface] = (input[surface] ?? []).map(rewrite);
const locators = input.locators ?? [];
const carrying = locators.flatMap((locator, index) =>
  terms.some((entry) => locator.value.includes(entry.term)) ? [index] : []);
let ordinal = 0;
answer.locators = locators.map((locator, index) => carrying.includes(index)
  ? { redacted: true, kind: locator.kind, ordinal: ++ordinal } : locator.value);
answer.replaced = replaced;
if (carrying.length > 0) {
  Object.assign(answer, { verdict: "refuse", surface: "locators",
    unfixed: carrying.map((index) => ({ surface: "locators", index, fixed: true })) });
  console.log(JSON.stringify(answer));
  process.exit(1);
}
Object.assign(answer, { verdict: "pass", unfixed: [] });
console.log(JSON.stringify(answer));
`;

const roots: string[] = [];

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
      },
      sources,
    }),
  );
  return [new OnetaskgraphClient({ binaryPath: binary, cwd: root }), root];
}

test("a search sweep answers its report on exit 3", async () => {
  const python = Bun.which("python3");
  if (python === null) throw new Error("python3 is needed to run the metered peer");
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
      config: { command: python, args: [peer, held] },
    },
  });
  // The store above was only for its directory; this one carries the peer.
  const report = await client.sourcesSanitize("tracker");
  expect(report.coverage).toEqual({
    complete: false,
    uncovered: ["metadata", "comments", "normalized-spellings"],
  });
  expect(report.items.map((item) => [item.id, item.outcome])).toEqual([
    ["tracker:t1", "rewritten"],
  ]);
  const after = JSON.parse(readFileSync(held, "utf8")).tasks;
  expect(after.find((entry: { id: string }) => entry.id === "t1").content).toBe("As ember-x does.");
});

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
