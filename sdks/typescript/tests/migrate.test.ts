import "./ambient.ts";
import { afterAll, expect, test } from "bun:test";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import {
  OnetaskgraphClient,
  OnetaskgraphExecutionError,
  type StoredFormsMigrated,
} from "../src/index.ts";

// `sources migrate` through the SDK: its report comes back on exit 0, and on exit 1 too. A folder
// of Markdown has only ever written one stored form, so against the real binary it answers that it
// has nothing to migrate and its files do not move. A migration that left an item unrewritten
// answers its report on exit 1, which only a Linear source produces — the Linear suite drives that
// against the loopback Linear — so a stand-in executable answers exactly the document the binary
// writes then, to hold the SDK's own half: the report is returned, not thrown.

const binary = resolve(import.meta.dir, "../../../target/debug/onetaskgraph");
const roots: string[] = [];

afterAll(() => {
  for (const root of roots) rmSync(root, { recursive: true, force: true });
});

// A store whose one source, `plan`, is a folder of Markdown holding one task.
function folder(): [OnetaskgraphClient, string] {
  const root = mkdtempSync(resolve(tmpdir(), "onetaskgraph-migrate-"));
  roots.push(root);
  const plan = resolve(root, "plan");
  mkdirSync(resolve(plan, "tasks"), { recursive: true });
  writeFileSync(
    resolve(plan, "tasks", "T-1.md"),
    '---\ntitle: Build it\nstatus: todo\nmetadata: {"caller.key": 1}\n---\nBody.\n',
  );
  writeFileSync(
    resolve(root, "onetaskgraph.yaml"),
    JSON.stringify({ sources: { plan: { plugin: "local-md", config: { root: plan } } } }),
  );
  return [new OnetaskgraphClient({ binaryPath: binary, cwd: root }), plan];
}

test("a folder of Markdown answers nothing to migrate, dry run or not, and moves nothing", async () => {
  const [client, plan] = folder();
  const task = resolve(plan, "tasks", "T-1.md");
  const before = readFileSync(task, "utf8");
  for (const dryRun of [true, false]) {
    expect(await client.sourcesMigrate("plan", { dryRun })).toEqual({
      source: "plan",
      dry_run: dryRun,
      rewritten: [],
      current: 0,
      failed: [],
    });
  }
  expect(readFileSync(task, "utf8")).toBe(before);
});

test("an unknown source is thrown as the failure it is, and bad arguments never run", async () => {
  const [client] = folder();
  const refused = await client.sourcesMigrate("elsewhere").catch((error: unknown) => error);
  expect(refused).toBeInstanceOf(OnetaskgraphExecutionError);
  expect((refused as OnetaskgraphExecutionError).exitCode).toBe(2);
  expect(() => client.sourcesMigrate(7 as unknown as string)).toThrow(TypeError);
  expect(() => client.sourcesMigrate("plan", { dryRun: "yes" as unknown as boolean })).toThrow(
    TypeError,
  );
});

// What `sources migrate work --json` writes when one item was rewritten and Linear refused
// another, exit 1 — the shape `collapsed_slot.rs` in the Linear suite drives the binary to.
const FAILED: StoredFormsMigrated = {
  source: "work",
  dry_run: false,
  rewritten: [{ kind: "task", id: "work:I-1", from: "html-comment-slot" }],
  current: 3,
  failed: [
    {
      kind: "document",
      id: "work:D-1",
      from: "multi-line-comment-slot",
      reason: "Linear reported documentUpdate was unsuccessful",
    },
  ],
};

test.skipIf(process.platform === "win32")(
  "a migration that left an item unrewritten returns its report on exit 1",
  async () => {
    const root = mkdtempSync(resolve(tmpdir(), "onetaskgraph-migrate-"));
    roots.push(root);
    const standIn = resolve(root, "onetaskgraph");
    writeFileSync(
      standIn,
      `#!${process.execPath}\n` +
        "const args = process.argv.slice(2);\n" +
        'if (args.slice(0, 3).join(" ") !== "sources migrate work") process.exit(9);\n' +
        `console.log(${JSON.stringify(JSON.stringify(FAILED))});\n` +
        "process.exit(1);\n",
    );
    chmodSync(standIn, 0o755);
    const client = new OnetaskgraphClient({ binaryPath: standIn, cwd: root });
    expect(await client.sourcesMigrate("work")).toEqual(FAILED);
  },
);
