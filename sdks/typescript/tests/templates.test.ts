import "./ambient.ts";
import { afterAll, beforeAll, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { OnetaskgraphClient, OnetaskgraphExecutionError } from "../src/index.ts";

const binary = resolve(import.meta.dir, "../../../target/debug/onetaskgraph");

const TASK = `---
onetaskgraph_template: 1
variables:
  title:
    description: What the task is called
  steps:
    description: What to do
    type: list
  size:
    description: How big
    type: integer
    default: 1
---
{% extends "base.md" %}
{% block body %}
{{ title }} ({{ size }})
{% for step in steps %}
- {{ step }}
{% endfor %}
{% endblock %}
`;

const BASE = `---
onetaskgraph_template: 1
variables:
  owner:
    description: Who owns it
    default: nobody
---
# Owned by {{ owner }}
{% block body %}{% endblock %}
`;

let root = "";
let task = "";
let library = "";
let client: OnetaskgraphClient;

beforeAll(() => {
  root = mkdtempSync(resolve(tmpdir(), "onetaskgraph-templates-"));
  library = resolve(root, "library");
  mkdirSync(library);
  writeFileSync(resolve(library, "base.md"), BASE);
  task = resolve(root, "task.md");
  writeFileSync(task, TASK);
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

test("templateVariables reads the declared set down the chain with its digest", async () => {
  const described = await client.templateVariables(task, { searchPath: [library] });
  expect(described.template).toBe("task.md");
  expect(described.digest).toMatch(/^sha256:[0-9a-f]{64}$/);
  expect(
    described.variables.map((variable) => [variable.name, variable.type, variable.declared_in]),
  ).toEqual([
    ["title", "string", "task.md"],
    ["steps", "list", "task.md"],
    ["size", "integer", "task.md"],
    ["owner", "string", "base.md"],
  ]);
  expect(described.variables[1]?.items).toBe("string");
  expect(described.variables[3]?.default).toBe("nobody");
});

test("templateRender hands the answers over on stdin, lets vars outrank them, and never prompts", async () => {
  const rendered = await client.templateRender(task, {
    searchPath: [library],
    answers: { title: "Ship it", steps: ["build", "release"], size: 3 },
    vars: { size: "5" },
  });
  // `owner` is answered by nothing and prompting is on in the environment, so a call that
  // could prompt would be refused for having no terminal; the default is taken instead.
  expect(rendered.body).toBe("# Owned by nobody\nShip it (5)\n- build\n- release\n");
  expect(rendered.answers).toEqual({
    title: "Ship it",
    steps: ["build", "release"],
    size: 5,
    owner: "nobody",
  });
  const described = await client.templateVariables(task, { searchPath: [library] });
  expect(rendered.digest).toBe(described.digest);
});

test("a refused answer is an execution error carrying exit 2 and what it refuses", async () => {
  const missing = await client
    .templateRender(task, { searchPath: [library] })
    .then(() => undefined)
    .catch((error: unknown) => error);
  if (!(missing instanceof OnetaskgraphExecutionError)) {
    throw new Error(`expected an execution error, received ${String(missing)}`);
  }
  expect(missing.exitCode).toBe(2);
  expect(missing.stderr).toContain("required variables are unanswered: title, steps");

  const mistyped = client.templateRender(task, {
    searchPath: [library],
    answers: { title: "t", steps: "not a list" },
  });
  await expect(mistyped).rejects.toThrow('"steps" is not a list');
});
