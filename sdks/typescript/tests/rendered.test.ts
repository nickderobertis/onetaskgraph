import "./ambient.ts";
import { afterAll, beforeAll, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { OnetaskgraphClient, OnetaskgraphExecutionError } from "../src/index.ts";

// Tasks and documents created from a template, regenerated and read back through the SDK,
// driving the real binary over a real folder of Markdown — which keeps the answers an item was
// rendered from in its own file.

const binary = resolve(import.meta.dir, "../../../target/debug/onetaskgraph");

const TASK = `---
onetaskgraph_template: 1
variables:
  goal:
    description: What the task is for
  steps:
    description: How it is done
    type: list
    default: []
---
Goal: {{ goal }}
{% for step in steps %}
- {{ step }}
{% endfor %}
`;

let root = "";
let template = "";
let client: OnetaskgraphClient;

beforeAll(() => {
  root = mkdtempSync(resolve(tmpdir(), "onetaskgraph-rendered-"));
  mkdirSync(resolve(root, "notes"));
  writeFileSync(
    resolve(root, "onetaskgraph.yaml"),
    JSON.stringify({ sources: { notes: { plugin: "local-md", config: { root: "notes" } } } }),
  );
  template = resolve(root, "task.md");
  writeFileSync(template, TASK);
  // Prompting on in the environment: the client has to override it on every call.
  client = new OnetaskgraphClient({
    binaryPath: binary,
    cwd: root,
    env: { ONETASKGRAPH_INTERACTIVE: "true" },
  });
});

afterAll(() => {
  rmSync(root, { recursive: true, force: true });
});

function sha256(text: string): string {
  return `sha256:${createHash("sha256").update(text, "utf8").digest("hex")}`;
}

test("taskCreate renders from answers on stdin, records provenance and stores the answers", async () => {
  const created = await client.taskCreate("notes", "P-1", "Ship it", {
    template,
    answers: { goal: "Ship it", steps: ["build"] },
    labels: ["release"],
    metadata: { "myapp.estimate": 3 },
  });
  const task = created.items[0];
  expect(task?.item.content).toBe("Goal: Ship it\n- build\n");
  expect(task?.item.metadata?.["myapp.estimate"]).toBe(3);
  const provenance = task?.item.metadata?.["onetaskgraph.template"] as Record<string, string>;
  expect(provenance.template).toBe(template);
  expect(provenance.body_digest).toBe(sha256("Goal: Ship it\n- build\n"));
  expect(provenance.answers_digest).toBe(sha256('{"goal":"Ship it","steps":["build"]}'));

  const id = task?.id ?? "";
  expect(await client.taskAnswers(id)).toEqual({ goal: "Ship it", steps: ["build"] });

  const regenerated = await client.taskRender(id, { vars: { goal: "Ship it again" } });
  expect(regenerated.changed).toBe(true);
  expect(regenerated.body).toBe("Goal: Ship it again\n- build\n");
  expect(regenerated.digest).toBe(provenance.digest ?? "");
  expect((await client.taskRender(id)).changed).toBe(false);
  const dry = await client.taskRender(id, { answers: { goal: "Never" }, dryRun: true });
  expect(dry.changed).toBe(true);
  expect((await client.taskShow(id)).items[0]?.item.content).toBe("Goal: Ship it again\n- build\n");
});

test("a plain body goes over stdin, and a document is created, rendered and answered", async () => {
  const plain = await client.taskCreate("notes", "P-1", "By hand", { body: "Written by hand." });
  expect(plain.items[0]?.item.content).toBe("Written by hand.");
  expect(plain.items[0]?.item.metadata?.["onetaskgraph.template"]).toBeUndefined();
  const refused = await client.taskAnswers(plain.items[0]?.id ?? "").catch((error) => error);
  expect(refused).toBeInstanceOf(OnetaskgraphExecutionError);
  expect((refused as OnetaskgraphExecutionError).exitCode).toBe(1);

  const document = await client.documentCreate("notes", "P-1", "Design", {
    id: "design",
    template,
    vars: { goal: "Design" },
  });
  expect(document.items[0]?.id).toBe("notes:design");
  expect(await client.documentAnswers("notes:design")).toEqual({ goal: "Design", steps: [] });
  const regenerated = await client.documentRender("notes:design", { vars: { goal: "Again" } });
  expect(regenerated.body).toBe("Goal: Again\n");
});

test("answers out of step refuse a partial render with exit 2, and a full one lands", async () => {
  const created = await client.taskCreate("notes", "P-1", "Edited", {
    template,
    vars: { goal: "Ship" },
  });
  const id = created.items[0]?.id ?? "";
  const file = resolve(root, "notes/tasks/edited.md");
  writeFileSync(file, readFileSync(file, "utf8").replace("goal: Ship", "goal: Forged"));

  const refused = await client.taskRender(id, { vars: { steps: "[x]" } }).catch((error) => error);
  expect(refused).toBeInstanceOf(OnetaskgraphExecutionError);
  expect((refused as OnetaskgraphExecutionError).exitCode).toBe(2);
  expect((refused as OnetaskgraphExecutionError).stderr).toContain("supply every required answer");
  expect((await client.taskRender(id, { answers: { goal: "Ship" } })).changed).toBe(true);
});

test("a loader document names the template, and a body never rides beside answers", async () => {
  const loader = resolve(root, "loader.json");
  writeFileSync(
    loader,
    JSON.stringify({ reference: "caller:task", entry: "task.md", search_path: [root] }),
  );
  const described = await client.templateVariables(undefined, { templateLoader: loader });
  expect(described.variables.map((variable) => variable.name)).toEqual(["goal", "steps"]);
  const rendered = await client.templateRender(undefined, {
    templateLoader: loader,
    vars: { goal: "Loaded" },
  });
  expect(rendered.body).toBe("Goal: Loaded\n");
  const created = await client.taskCreate("notes", "P-1", "Loaded", {
    templateLoader: loader,
    vars: { goal: "x" },
  });
  const provenance = created.items[0]?.item.metadata?.["onetaskgraph.template"] as Record<
    string,
    string
  >;
  expect(provenance.template).toBe("caller:task");

  await expect(
    client.taskCreate("notes", "P-1", "Both", { template, body: "x", answers: { goal: "x" } }),
  ).rejects.toThrow("body and answers both go to standard input");
});
