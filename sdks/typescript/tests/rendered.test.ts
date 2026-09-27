import "./ambient.ts";
import { afterAll, beforeAll, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import {
  OnetaskgraphClient,
  OnetaskgraphExecutionError,
  type RenderOptions,
} from "../src/index.ts";

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

// The `onetaskgraph.template` entry of an item's metadata, checked field by field rather than
// asserted into shape.
function provenanceOf(metadata: { [key: string]: unknown } | null | undefined) {
  const entry: unknown = metadata?.["onetaskgraph.template"];
  if (typeof entry !== "object" || entry === null) throw new Error("no provenance recorded");
  const field = (name: string): string => {
    const value: unknown = Reflect.get(entry, name);
    if (typeof value !== "string") throw new Error(`provenance ${name} is not a string`);
    return value;
  };
  return {
    template: field("template"),
    digest: field("digest"),
    body_digest: field("body_digest"),
    answers_digest: field("answers_digest"),
  };
}

// The execution error a call was refused with, narrowed by the check rather than asserted.
async function refusal(call: Promise<unknown>): Promise<OnetaskgraphExecutionError> {
  try {
    await call;
  } catch (error) {
    if (error instanceof OnetaskgraphExecutionError) return error;
    throw error;
  }
  throw new Error("the call was expected to be refused");
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
  const provenance = provenanceOf(task?.item.metadata);
  expect(provenance.template).toBe(template);
  expect(provenance.body_digest).toBe(sha256("Goal: Ship it\n- build\n"));
  expect(provenance.answers_digest).toBe(sha256('{"goal":"Ship it","steps":["build"]}'));

  const id = task?.id ?? "";
  expect(await client.taskAnswers(id)).toEqual({ goal: "Ship it", steps: ["build"] });

  const regenerated = await client.taskRender(id, { vars: { goal: "Ship it again" } });
  expect(regenerated.changed).toBe(true);
  expect(regenerated.body).toBe("Goal: Ship it again\n- build\n");
  expect(regenerated.digest).toBe(provenance.digest);
  expect((await client.taskRender(id)).changed).toBe(false);
  const dry = await client.taskRender(id, { answers: { goal: "Never" }, dryRun: true });
  expect(dry.changed).toBe(true);
  expect((await client.taskShow(id)).items[0]?.item.content).toBe("Goal: Ship it again\n- build\n");
});

test("a plain body goes over stdin, and a document is created, rendered and answered", async () => {
  const plain = await client.taskCreate("notes", "P-1", "By hand", { body: "Written by hand." });
  expect(plain.items[0]?.item.content).toBe("Written by hand.");
  expect(plain.items[0]?.item.metadata?.["onetaskgraph.template"]).toBeUndefined();
  const refused = await refusal(client.taskAnswers(plain.items[0]?.id ?? ""));
  expect(refused.exitCode).toBe(1);

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

  const refused = await refusal(client.taskRender(id, { vars: { steps: "[x]" } }));
  expect(refused.exitCode).toBe(2);
  expect(refused.stderr).toContain("supply every required answer");
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
  expect(provenanceOf(created.items[0]?.item.metadata).template).toBe("caller:task");

  await expect(
    client.taskCreate("notes", "P-1", "Both", { template, body: "x", answers: { goal: "x" } }),
  ).rejects.toThrow("body and answers both go to standard input");
});

test("a path, a body or metadata the binary could not be handed is refused before it starts", async () => {
  await expect(client.taskCreate("notes", "P-1", "Refused", { template: "" })).rejects.toThrow(
    "taskCreate: template is not a path",
  );
  await expect(client.documentRender("notes:design", { templateLoader: "-" })).rejects.toThrow(
    "documentRender: templateLoader is not a path",
  );
  await expect(client.templateVariables(undefined, { templateLoader: "" })).rejects.toThrow(
    "templateVariables: templateLoader is not a path",
  );
  await expect(
    client.taskCreate("notes", "P-1", "Refused", { template, body: "ignored" }),
  ).rejects.toThrow("taskCreate: body is read only when no template");
  await expect(
    client.documentCreate("notes", "P-1", "Refused", { bodyFile: "b.md", body: "ignored" }),
  ).rejects.toThrow("documentCreate: body is read only when no template");
  await expect(
    client.taskCreate("notes", "P-1", "Refused", { bodyFile: "--json" }),
  ).rejects.toThrow("taskCreate: bodyFile is not a path");
  await expect(
    // Deliberately outside the declared type, as a caller whose values reached it untyped.
    client.taskCreate("notes", "P-1", "Refused", { body: 7 as unknown as string }),
  ).rejects.toThrow("taskCreate: body is not a string");
  await expect(
    client.taskCreate("notes", "P-1", "Refused", {
      body: "x",
      metadata: { "myapp.when": Number.NaN },
    }),
  ).rejects.toThrow("taskCreate: metadata.myapp.when is not a JSON value");
  await expect(
    // Outside the declared type on purpose, as the `body` above is.
    client.taskRender("notes:design", { unset: ["ok", 3 as unknown as string] }),
  ).rejects.toThrow("taskRender: unset[1] is not a string");
  await expect(
    // A list carrying a key besides its entries: iterating by index would drop it unsent.
    client.taskCreate("notes", "P-1", "Refused", {
      body: "x",
      labels: Object.assign(["kept"], { extra: "dropped" }),
    }),
  ).rejects.toThrow("taskCreate: labels has the key extra, which is not sent");
  // `as const` keeps each pair a tuple, so `render` is typed as the method it was bound from
  // rather than as a union of a name and a function.
  for (const [method, render] of [
    ["taskRender", client.taskRender.bind(client)],
    ["documentRender", client.documentRender.bind(client)],
  ] as const) {
    await expect(
      // Outside the declared type on purpose: a render has no body option, and one that
      // reached it untyped would be written to a standard input the binary never reads.
      render("notes:design", { body: "x" } as unknown as RenderOptions),
    ).rejects.toThrow(`${method}: body is not an option of a render`);
  }
});

test("every option of the create and render methods reaches the real binary as a flag it takes", async () => {
  // The binary refuses a flag it does not know, so one call per method with every option set is
  // what holds these option names to the command line they are spelled for.
  const blocker = await client.taskCreate("notes", "P-1", "Blocker", { body: "b" });
  const delivered = await client.taskCreate("notes", "P-1", "Delivered", { body: "d" });
  const everything = await client.taskCreate("notes", "P-1", "Every option", {
    template,
    searchPath: [root],
    answers: { goal: "All of it" },
    vars: { steps: "[one]" },
    status: "queued",
    labels: ["every"],
    repositories: ["github.com/acme/every"],
    metadata: { "myapp.every": [true, null] },
    dependsOn: [blocker.items[0]?.id ?? ""],
    delivers: [delivered.items[0]?.id ?? ""],
  });
  const task = everything.items[0];
  expect(task?.item.status.category).toBe("queued");
  expect(task?.item.repositories).toEqual(["github.com/acme/every"]);
  expect(task?.item.delivers).toEqual([delivered.items[0]?.id ?? ""]);
  const rendered = await client.taskRender(task?.id ?? "", {
    template,
    searchPath: [root],
    answers: { goal: "Again" },
    vars: { steps: "[two]" },
    unset: ["steps"],
    dryRun: true,
  });
  expect(rendered.body).toBe("Goal: Again\n");
  const bodyFile = resolve(root, "body.md");
  writeFileSync(bodyFile, "From a file.");
  const document = await client.documentCreate("notes", "P-1", "Every document option", {
    id: "every",
    bodyFile,
    labels: ["every"],
    repositories: ["github.com/acme/every"],
    metadata: { "myapp.every": 1 },
  });
  expect(document.items[0]?.item.content).toBe("From a file.");
  const regenerated = await client.documentRender(document.items[0]?.id ?? "", {
    template,
    searchPath: [root],
    answers: { goal: "Again" },
    vars: { steps: "[two]" },
    unset: ["steps"],
    dryRun: true,
  });
  expect(regenerated.body).toBe("Goal: Again\n");
  expect(regenerated.changed).toBe(true);
  await expect(
    // Deliberately outside the declared type, as a caller whose values reached it untyped.
    client.taskRender(task?.id ?? "", [] as unknown as RenderOptions),
  ).rejects.toThrow("taskRender: options is not a plain object");
  await expect(
    // Outside the declared type on purpose, as the options above are: a truthy non-boolean
    // must not become `--dry-run`.
    client.taskRender(task?.id ?? "", { dryRun: "no" as unknown as boolean }),
  ).rejects.toThrow("taskRender: dryRun is not a boolean");
});

test("every flag the create, render, answers and template verbs take is spelled in the client", () => {
  // The other half of the test above, checked coarsely: a flag the binary adds to one of these
  // verbs fails here unless `client.ts` spells it somewhere, which is where a caller's option
  // would have to become it. Which method spells it is the test above's to prove, by driving
  // each option to the real binary. Global flags are the client's own business — it always
  // passes `--json` and `--no-interactive` — and are not options of any one call.
  const global = new Set([
    "set",
    "page-size",
    "default-sources",
    "output",
    "json",
    "interactive",
    "no-interactive",
    "help",
  ]);
  const client = readFileSync(resolve(import.meta.dir, "../src/client.ts"), "utf8");
  for (const command of [
    "task create",
    "task render",
    "task answers",
    "document create",
    "document render",
    "document answers",
    "template variables",
    "template render",
  ]) {
    const help = spawnSync(binary, [...command.split(" "), "--help"], { encoding: "utf8" }).stdout;
    const flags = [...help.matchAll(/^\s+--([a-z][a-z-]*)/gm)].map(([, flag]) => flag ?? "");
    expect(flags.length).toBeGreaterThan(0);
    for (const flag of flags.filter((flag) => !global.has(flag))) {
      expect(
        client.includes(`"--${flag}"`),
        `${command} --${flag} is not spelled by the client`,
      ).toBe(true);
    }
  }
});
