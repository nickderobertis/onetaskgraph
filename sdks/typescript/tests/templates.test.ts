import "./ambient.ts";
import { afterAll, beforeAll, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import {
  type JsonValue,
  OnetaskgraphClient,
  OnetaskgraphExecutionError,
  type TemplateOptions,
  type TemplateRenderOptions,
} from "../src/index.ts";

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

test("answers JSON cannot carry are refused before the binary is started", async () => {
  const cyclic: Record<string, unknown> = { title: "t" };
  cyclic.self = cyclic;
  // A hole, which JSON.stringify would otherwise write as `null`: assigned past the end
  // rather than spelled as a literal, which is how one usually arises.
  const sparse: unknown[] = ["a"];
  sparse[2] = "c";
  const refusals: [unknown, string][] = [
    [{ title: "t", steps: [Number.NaN] }, "answers.steps[0]"],
    [{ title: undefined }, "answers.title"],
    [{ title: "t", when: new Date(0) }, "answers.when"],
    [cyclic, "answers.self"],
    [{ title: "t", steps: sparse }, "answers.steps[1]"],
  ];
  for (const [answers, path] of refusals) {
    const refused = client.templateRender(task, {
      searchPath: [library],
      // Deliberately outside the declared type: this is the runtime half of that boundary,
      // for a caller whose values reached it untyped.
      answers: answers as Record<string, JsonValue>,
    });
    await expect(refused).rejects.toThrow(TypeError);
    await expect(refused).rejects.toThrow(`${path} is not a JSON value`);
  }
});

test("an answer key JSON cannot carry is refused rather than dropped", async () => {
  const hidden = { title: "t" };
  Object.defineProperty(hidden, "steps", { value: ["a"], enumerable: false });
  // An array carrying its own `toJSON`: serialising the caller's object would send "swapped",
  // and copying its entries alone would drop what it carries.
  const swapping = ["checked"];
  Object.defineProperty(swapping, "toJSON", { value: () => ["swapped"] });
  const labelled = Object.assign(["a"], { note: "b" });
  // A key spelled like an index but past the largest one an array has: an ordinary property,
  // so `length` does not reach it and copying by index would leave it behind.
  const beyond: unknown[] = ["a"];
  (beyond as unknown as Record<string, unknown>)["4294967295"] = "b";
  const refusals: [Record<string, unknown>, string][] = [
    [{ title: "t", [Symbol("steps")]: ["a"] }, "answers has the key Symbol(steps)"],
    [{ title: "t", nested: { [Symbol("x")]: 1 } }, "answers.nested has the key Symbol(x)"],
    [hidden, "answers has the key steps"],
    [{ title: "t", steps: swapping }, "answers.steps has the key toJSON"],
    [{ title: "t", steps: labelled }, "answers.steps has the key note"],
    [{ title: "t", steps: beyond }, "answers.steps has the key 4294967295"],
  ];
  for (const [answers, message] of refusals) {
    const refused = client.templateRender(task, {
      searchPath: [library],
      // Widened from `unknown` values: a symbol key or an array's own `toJSON` is what the
      // declared type cannot express, and this is the runtime half of that boundary.
      answers: answers as Record<string, JsonValue>,
    });
    await expect(refused).rejects.toThrow(TypeError);
    await expect(refused).rejects.toThrow(message);
  }

  const hiddenVar = { title: "t" };
  Object.defineProperty(hiddenVar, "size", { value: "5", enumerable: false });
  const varRefusals: [Record<string, string>, string][] = [
    [
      // A symbol key is outside what `Record<string, string>` admits: this is the runtime half
      // of that boundary, for a caller whose mapping reached it untyped.
      { title: "t", [Symbol("size")]: "5" } as Record<string, string>,
      "vars has the key Symbol(size)",
    ],
    [hiddenVar, "vars has the key size"],
  ];
  for (const [vars, message] of varRefusals) {
    const refused = client.templateRender(task, {
      searchPath: [library],
      answers: { steps: [] },
      vars,
    });
    await expect(refused).rejects.toThrow(TypeError);
    await expect(refused).rejects.toThrow(message);
  }
});

test("what is sent is the answers checked, and a var is text or refused", async () => {
  const rendered = await client.templateRender(task, {
    searchPath: [library],
    answers: { title: "t", steps: ["checked"] },
  });
  expect(rendered.answers.steps).toEqual(["checked"]);
  expect(rendered.body).toContain("- checked");

  const refused = client.templateRender(task, {
    searchPath: [library],
    answers: { title: "t", steps: [] },
    // Deliberately outside the declared type, as a caller whose values reached it untyped.
    vars: { size: 5 } as unknown as Record<string, string>,
  });
  await expect(refused).rejects.toThrow(TypeError);
  await expect(refused).rejects.toThrow("vars.size is not a string");
});

test("a search path entry that is not a path string is refused before the binary is started", async () => {
  // Deliberately outside the declared type, as a caller whose values reached it untyped.
  const searchPath = [library, 7] as unknown as string[];
  await expect(client.templateVariables(task, { searchPath })).rejects.toThrow(
    "searchPath[1] is not a string",
  );
  await expect(client.templateRender(task, { searchPath })).rejects.toThrow(TypeError);

  // A hole, assigned past the end: refused rather than skipped.
  const sparse: string[] = [library];
  sparse[2] = library;
  await expect(client.templateVariables(task, { searchPath: sparse })).rejects.toThrow(
    "searchPath[1] is not a string",
  );
});

test("a template file that is not a path string is refused before the binary is started", async () => {
  // Deliberately outside the declared type, as a caller whose values reached it untyped.
  for (const file of [7, undefined, null, "", "--json", "-"]) {
    const path = file as unknown as string;
    await expect(client.templateVariables(path)).rejects.toThrow(
      "templateVariables: file is not a template path",
    );
    await expect(client.templateRender(path, { searchPath: [library] })).rejects.toThrow(
      "templateRender: file is not a template path",
    );
  }

  // The remedy the refusal names: the same file spelled from the directory it is in.
  const dashed = resolve(root, "-dashed.md");
  writeFileSync(dashed, "dashed\n");
  const rendered = await new OnetaskgraphClient({ binaryPath: binary, cwd: root }).templateRender(
    "./-dashed.md",
  );
  expect(rendered.body).toBe("dashed\n");
});

test("answers that are not a mapping are refused before the binary is started", async () => {
  for (const answers of [["title", "t"], null, new Map([["title", "t"]])]) {
    const refused = client.templateRender(task, {
      searchPath: [library],
      // Deliberately outside the declared type, as a caller whose values reached it untyped.
      answers: answers as unknown as Record<string, JsonValue>,
    });
    await expect(refused).rejects.toThrow("answers is not a plain object");
  }
});

test("options that are not an object are refused as options, not as a property read", async () => {
  // Deliberately outside the declared types, as a caller whose values reached it untyped.
  for (const options of [null, "searchPath", 7, [library], new Map()]) {
    const refused = [
      client.templateVariables(task, options as unknown as TemplateOptions),
      client.templateRender(task, options as unknown as TemplateRenderOptions),
    ];
    for (const call of refused) {
      await expect(call).rejects.toThrow(TypeError);
      await expect(call).rejects.toThrow("options is not a plain object");
    }
  }
});

test("a search path that is not a list and vars that are not a mapping are refused", async () => {
  // Deliberately outside the declared types, as a caller whose values reached it untyped.
  const searchPath = library as unknown as string[];
  await expect(client.templateVariables(task, { searchPath })).rejects.toThrow(
    "searchPath is not an array",
  );
  await expect(
    client.templateVariables(task, { searchPath: null as unknown as string[] }),
  ).rejects.toThrow("searchPath is not an array");
  for (const vars of [["title=t"], "title=t", 7, null]) {
    await expect(
      client.templateRender(task, {
        searchPath: [library],
        vars: vars as unknown as Record<string, string>,
      }),
    ).rejects.toThrow("vars is not a plain object");
  }
});
