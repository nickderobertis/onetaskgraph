import "./ambient.ts";
import { afterAll, beforeAll, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { OnetaskgraphClient, OnetaskgraphExecutionError } from "../src/index.ts";

// Content-keyed attestations recorded and checked through the SDK, driving the real binary over
// a real folder of Markdown; every answer is validated against the root the binary emits for it,
// a check that is not valid (exit 3) included.

const binary = resolve(import.meta.dir, "../../../target/debug/onetaskgraph");

let root = "";
let client: OnetaskgraphClient;
let task = "";

beforeAll(async () => {
  root = mkdtempSync(resolve(tmpdir(), "onetaskgraph-attestations-"));
  mkdirSync(resolve(root, "notes"));
  writeFileSync(
    resolve(root, "onetaskgraph.yaml"),
    JSON.stringify({ sources: { notes: { plugin: "local-md", config: { root: "notes" } } } }),
  );
  client = new OnetaskgraphClient({ binaryPath: binary, cwd: root });
  await client.projectCreate("notes", "P-1", "Plan", { body: "The plan.\n" });
  const created = await client.taskCreate("notes", "P-1", "Build", { body: "Build it.\n" });
  task = created.items[0]?.id ?? "";
});

afterAll(() => {
  rmSync(root, { recursive: true, force: true });
});

test("an attestation is recorded, holds, and is void once the content it cleared moves", async () => {
  expect((await client.taskAttestationCheck(task, "plan-review")).state).toBe("absent");
  const attested = await client.taskAttest(task, "plan-review", "ada", "bar-1", {
    note: "fine",
  });
  expect("record" in attested && attested.record.bar).toBe("bar-1");
  const valid = await client.taskAttestationCheck(task, "plan-review", { bar: "bar-1" });
  expect([valid.state, valid.reason]).toEqual(["valid", null]);
  const moved = await client.taskAttestationCheck(task, "plan-review", { bar: "bar-2" });
  expect([moved.state, moved.reason]).toEqual(["void", "bar-moved"]);
  await client.taskMetadataSet(task, "myapp.size", "2");
  const content = await client.taskAttestationCheck(task, "plan-review", { bar: "bar-1" });
  expect([content.state, content.reason]).toEqual(["void", "content-moved"]);

  await client.projectAttest("notes:P-1", "plan-review", "ada", "bar-1");
  const tasks = await client.projectAttestationCheck("notes:P-1", "plan-review", { tasks: true });
  expect(
    "tasks" in tasks &&
      tasks.tasks.map((entry: { id: string; state: string }) => [entry.id, entry.state]),
  ).toEqual([[task, "void"]]);
  const own = await client.projectAttestationCheck("notes:P-1", "plan-review");
  expect("state" in own && own.state).toBe("valid");
});

test("a kind the binary refuses rejects with its failure", async () => {
  const refused = client.taskAttest(task, "two words", "ada", "bar-1");
  await expect(refused).rejects.toBeInstanceOf(OnetaskgraphExecutionError);
});
