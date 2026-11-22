import "./ambient.ts";
import { afterAll, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { OnetaskgraphClient, OnetaskgraphExecutionError } from "../src/index.ts";

// `sources quota` and a duplicate close through the SDK, against the real binary over a folder of
// Markdown: a folder reports no reading, and a duplicate close leaves the task `cancelled` with
// its `duplicate` link at both ends. The hosted backends' own answers are the Rust suites'.

const binary = resolve(import.meta.dir, "../../../target/debug/onetaskgraph");
const roots: string[] = [];

afterAll(() => {
  for (const root of roots) rmSync(root, { recursive: true, force: true });
});

// A store whose one source, `plan`, is a folder of Markdown holding T-1 and T-2.
function folder(): [OnetaskgraphClient, string] {
  const root = mkdtempSync(resolve(tmpdir(), "onetaskgraph-store-fixes-"));
  roots.push(root);
  const plan = resolve(root, "plan");
  mkdirSync(resolve(plan, "tasks"), { recursive: true });
  for (const n of [1, 2]) {
    writeFileSync(
      resolve(plan, "tasks", `T-${n}.md`),
      `---\ntitle: Task ${n}\nstatus: todo\n---\nTask ${n}.\n`,
    );
  }
  writeFileSync(
    resolve(root, "onetaskgraph.yaml"),
    JSON.stringify({ sources: { plan: { plugin: "local-md", config: { root: plan } } } }),
  );
  return [new OnetaskgraphClient({ binaryPath: binary, cwd: root }), plan];
}

test("sourcesQuota answers a folder's empty reading and refuses an unknown source", async () => {
  const [client] = folder();
  expect(await client.sourcesQuota("plan")).toEqual({ source: "plan", resources: [] });
  const refused = await client.sourcesQuota("elsewhere").catch((error: unknown) => error);
  expect(refused).toBeInstanceOf(OnetaskgraphExecutionError);
  // Deliberately outside the declared type, as a JavaScript caller's value reaches it untyped:
  // the SDK's own check at the boundary is what refuses it, before the binary is started.
  expect(() => client.sourcesQuota(7 as unknown as string)).toThrow(TypeError);
});

test("taskStatusSet with duplicateOf closes the task cancelled with its duplicate link", async () => {
  const [client, plan] = folder();
  const set = await client.taskStatusSet("plan:T-1", "cancelled", { duplicateOf: "plan:T-2" });
  expect(set.status.category).toBe("cancelled");
  expect(readFileSync(resolve(plan, "tasks", "T-1.md"), "utf8")).toContain("status: cancelled");
  expect(readFileSync(resolve(plan, "tasks", "T-2.md"), "utf8")).toContain("## Links");
  // Beside another category the binary refuses it, writing nothing.
  const before = readFileSync(resolve(plan, "tasks", "T-2.md"), "utf8");
  const refused = await client
    .taskStatusSet("plan:T-2", "done", { duplicateOf: "plan:T-1" })
    .catch((error: unknown) => error);
  expect(refused).toBeInstanceOf(OnetaskgraphExecutionError);
  expect(readFileSync(resolve(plan, "tasks", "T-2.md"), "utf8")).toBe(before);
  // Deliberately outside the declared type, as above: a survivor that reached the SDK untyped.
  expect(() =>
    client.taskStatusSet("plan:T-1", "cancelled", { duplicateOf: 7 as unknown as string }),
  ).toThrow(TypeError);
});

test("taskUpdate with duplicateOf closes the task as a duplicate", async () => {
  const [client] = folder();
  const updated = await client.taskUpdate("plan:T-1", { duplicateOf: "plan:T-2" });
  expect(updated.task.status.category).toBe("cancelled");
  expect(updated.written).toContain("duplicate-of");
});
