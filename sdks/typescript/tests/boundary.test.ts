import "./ambient.ts";
import { afterAll, beforeAll, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { OnetaskgraphClient, OnetaskgraphExecutionError } from "../src/index.ts";

// The public boundary through the SDK: every write's `termScope` reaches the store's check
// exactly as the caller passed it, and the store's refusals reach the caller. The binary is the
// real one, over real folders of Markdown; the check command records what the store hands it,
// which is what makes "unchanged" something a test can read.

const binary = resolve(import.meta.dir, "../../../target/debug/onetaskgraph");

// Records each input it is handed, one per line, and passes it — or refuses it, the way a
// check refuses, when the text carries `REFUSE`.
const CHECK = `
const fs = require("node:fs");
const input = fs.readFileSync(0, "utf8");
fs.appendFileSync(process.env.BOUNDARY_RECORD, input.replace(/\\n/g, " ") + "\\n");
if (input.includes("REFUSE")) {
  console.log(JSON.stringify({ verdict: "refuse", surface: "text" }));
  process.exit(1);
}
console.log(JSON.stringify({ verdict: "pass" }));
`;

let root = "";
let record = "";
let client: OnetaskgraphClient;

beforeAll(() => {
  root = mkdtempSync(resolve(tmpdir(), "onetaskgraph-boundary-"));
  for (const folder of ["site", "plan"]) mkdirSync(resolve(root, folder));
  record = resolve(root, "checked.jsonl");
  writeFileSync(
    resolve(root, "onetaskgraph.yaml"),
    JSON.stringify({
      write_policy: { check_command: [process.execPath, "-e", CHECK] },
      sources: {
        site: { plugin: "local-md", config: { root: "site" }, visibility: "public" },
        plan: { plugin: "local-md", config: { root: "plan" }, visibility: "private" },
      },
    }),
  );
  client = new OnetaskgraphClient({
    binaryPath: binary,
    cwd: root,
    env: { BOUNDARY_RECORD: record },
  });
});

afterAll(() => {
  rmSync(root, { recursive: true, force: true });
});

// The scope of each input the check was handed since `from`, `undefined` where none was sent.
function scopes(from: number): (string[] | undefined)[] {
  return readFileSync(record, "utf8")
    .split("\n")
    .filter((line) => line.length > 0)
    .slice(from)
    .map((line) => {
      const input: unknown = JSON.parse(line);
      if (typeof input !== "object" || input === null) throw new Error("not an input");
      const scope: unknown = Reflect.get(input, "scope");
      if (scope === undefined) return undefined;
      if (!Array.isArray(scope)) throw new Error("scope is not a list");
      return scope.map(String);
    });
}

function checked(): number {
  try {
    return readFileSync(record, "utf8")
      .split("\n")
      .filter((line) => line.length > 0).length;
  } catch {
    return 0;
  }
}

test("every write's termScope reaches the store's check unchanged", async () => {
  const before = checked();
  const named = await client.taskCreate("site", "p", "Named", {
    body: "Generic.",
    termScope: ["github.com/example-org/widget", "github.com/example-org/gadget"],
  });
  const id = String(named.items[0]?.id);
  await client.taskCreate("site", "p", "Empty", { body: "Generic.", termScope: [] });
  await client.taskCreate("site", "p", "Unscoped", { body: "Generic." });
  await client.taskStatusSet(id, "done", { termScope: ["github.com/example-org/widget"] });
  await client.taskMetadataSet(id, "team.note", '"generic"', { termScope: [] });
  await client.taskUpdate(id, { title: "Renamed", termScope: ["github.com/example-org/widget"] });
  await client.taskCommentAdd(id, { body: "A note.", termScope: [] });
  expect(scopes(before)).toEqual([
    ["github.com/example-org/widget", "github.com/example-org/gadget"],
    [],
    undefined,
    ["github.com/example-org/widget"],
    [],
    ["github.com/example-org/widget"],
    [],
  ]);
});

test("a copy and a create into a private source carry their classification", async () => {
  const before = checked();
  const secret = await client.taskCreate("plan", "p", "Secret", {
    body: "Generic.",
    classification: "private",
  });
  expect(secret.items[0]?.item.classification).toBe("private");
  // A private source is not checked for terms, and a private item is never sent to a public one.
  expect(checked()).toBe(before);
  const refused = await client
    .taskCopy([String(secret.items[0]?.id)], "site", { termScope: [] })
    .then(
      () => undefined,
      (error: unknown) => error,
    );
  expect(refused).toBeInstanceOf(OnetaskgraphExecutionError);
  expect(String((refused as Error).message)).toContain("not declared private");
  const route = await client.sourcesRoute("site", { classification: "private" });
  expect(route.destination).toBe("site");
});

test("the check's refusal reaches the caller and nothing is written", async () => {
  const refused = await client
    .taskCreate("site", "p", "Refused", { body: "REFUSE this", termScope: [] })
    .then(
      () => undefined,
      (error: unknown) => error,
    );
  expect(refused).toBeInstanceOf(OnetaskgraphExecutionError);
  expect(String((refused as Error).message)).toContain("term of a private repository in its text");
  const listed = await client.taskList({ sources: ["site"], search: "Refused" });
  expect(listed.items).toEqual([]);
});
