import { afterAll, beforeAll, expect, test } from "bun:test";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, resolve } from "node:path";
import {
  OnetaskgraphClient,
  OnetaskgraphExecutionError,
  OnetaskgraphValidationError,
  assertCompleteCommandSurface,
  clientCommands,
} from "../src/index.ts";

const binary = resolve(import.meta.dir, "../../../target/debug/onetaskgraph");
let root = "";
let client: OnetaskgraphClient;

function executableFixture(directory: string, name: string, stdout: string, stderr = "", code = 0) {
  const windows = process.platform === "win32";
  const program = resolve(directory, `${name}.js`);
  const path = windows ? resolve(directory, `${name}.cmd`) : program;
  const quote = (value: string) => JSON.stringify(value);
  writeFileSync(
    program,
    `#!/usr/bin/env node\nprocess.stdout.write(${quote(stdout)});\nprocess.stderr.write(${quote(stderr)});\nprocess.exit(${code});\n`,
  );
  if (windows) writeFileSync(path, `@echo off\r\nnode "%~dp0${name}.js"\r\n`);
  else chmodSync(path, 0o755);
  return path;
}

function errorMessage(error: unknown): string {
  if (!(error instanceof Error)) throw new Error(`expected Error, received ${String(error)}`);
  return error.message;
}

beforeAll(() => {
  root = mkdtempSync(resolve(tmpdir(), "onetaskgraph-sdk-"));
  writeFileSync(
    resolve(root, "onetaskgraph.yaml"),
    JSON.stringify({
      sources: {
        work: {
          plugin: "in-memory",
          config: {
            tasks: [
              {
                id: "T-1",
                title: "Alpha engine",
                status: { category: "todo", name: "Todo" },
                labels: [{ id: "L-1", name: "bug" }],
                project: "P-1",
              },
              { id: "T-2", title: "Beta", status: { category: "done", name: "Done" }, labels: [] },
            ],
            projects: [
              {
                id: "P-1",
                title: "Engine",
                status: { category: "in-progress", name: "Doing" },
                labels: [{ id: "L-1", name: "bug" }],
              },
              { id: "P-2", title: "Docs", status: { category: "todo", name: "Todo" }, labels: [] },
            ],
            labels: [{ id: "L-1", name: "bug" }],
            task_dependencies: [{ from: "T-1", to: "T-2", kind: "blocks" }],
            project_dependencies: [{ from: "P-1", to: "P-2", kind: "blocks" }],
          },
        },
      },
    }),
  );
  client = new OnetaskgraphClient({ binaryPath: binary, cwd: root });
});

afterAll(() => rmSync(root, { recursive: true, force: true }));

test("every emitted command has a client method", async () => {
  assertCompleteCommandSurface();
  const bundle = await client.schema();
  expect(bundle).toHaveProperty("commands", [...clientCommands]);
});

test("typed methods drive every real binary command", async () => {
  expect((await client.configShow()).settings.length).toBeGreaterThan(0);
  expect((await client.sourcesList())[0]?.source).toBe("work");
  const tasks = await client.taskList({
    sources: ["work"],
    labels: ["bug"],
    excludeLabels: ["chore"],
    statuses: ["todo"],
    search: "Alpha",
    fields: "title",
    limit: 1,
    project: "P-1",
  });
  expect(tasks.items[0]?.id).toBe("work:T-1");
  const firstPage = await client.taskList({ sources: ["work"], limit: 1 });
  expect(firstPage.next).toBeString();
  const secondPage = await client.taskList({
    sources: ["work"],
    limit: 1,
    page: firstPage.next ?? "missing-page-token",
  });
  expect(secondPage.items).toHaveLength(1);
  expect(secondPage.items[0]?.id).not.toBe(firstPage.items[0]?.id);
  expect((await client.taskList({ sources: ["work"], noProject: true })).items[0]?.id).toBe(
    "work:T-2",
  );
  expect((await client.taskShow("work:T-1", { allowPartial: true })).items[0]?.item.title).toBe(
    "Alpha engine",
  );
  expect(
    (
      await client.taskDeps("work:T-2", {
        direction: "depended-on-by",
        limit: 1,
        allowPartial: true,
      })
    ).items[0]?.from,
  ).toEqual({ id: "work:T-1", kind: "task" });
  expect(
    (
      await client.projectList({
        sources: ["work"],
        labels: ["bug"],
        excludeLabels: ["chore"],
        statuses: ["in-progress"],
        search: "Engine",
        fields: "title",
        limit: 1,
        allowPartial: true,
      })
    ).items[0]?.id,
  ).toBe("work:P-1");
  expect((await client.projectShow("work:P-1", { allowPartial: true })).items[0]?.item.title).toBe(
    "Engine",
  );
  expect(
    (
      await client.projectDeps("work:P-2", {
        direction: "depended-on-by",
        limit: 1,
        allowPartial: true,
      })
    ).items[0]?.from,
  ).toEqual({ id: "work:P-1", kind: "project" });
  expect(
    (await client.labelList({ sources: ["work"], limit: 1, allowPartial: true })).items[0]?.id,
  ).toBe("work:L-1");
  expect(
    (
      await client.search("Alpha", {
        sources: ["work"],
        fields: "title",
        kind: "task",
        limit: 1,
        allowPartial: true,
      })
    ).items[0]?.kind,
  ).toBe("task");
});

test("copy drives the real binary and reports what it did to each item", async () => {
  const copyRoot = mkdtempSync(resolve(tmpdir(), "onetaskgraph-sdk-copy-"));
  mkdirSync(resolve(copyRoot, "from/tasks"), { recursive: true });
  mkdirSync(resolve(copyRoot, "into"), { recursive: true });
  writeFileSync(
    resolve(copyRoot, "from/tasks/T-1.md"),
    "---\ntitle: Alpha engine\nstatus: todo\nmetadata: {caller.count: 3}\n---\nthe engine core\n",
  );
  const folder = (root: string) => ({
    plugin: "local-md",
    config: { root: resolve(copyRoot, root), status_mapping: { todo: "todo" } },
  });
  writeFileSync(
    resolve(copyRoot, "onetaskgraph.yaml"),
    JSON.stringify({ sources: { from: folder("from"), into: folder("into") } }),
  );
  try {
    const copyClient = new OnetaskgraphClient({ binaryPath: binary, cwd: copyRoot });

    const planned = await copyClient.taskCopy(["from:T-1"], "into", { dryRun: true });
    expect(planned.items).toEqual([{ source: "from:T-1", destination: null, action: "created" }]);

    const created = await copyClient.taskCopy(["from:T-1"], "into");
    expect(created.items).toEqual([
      { source: "from:T-1", destination: "into:T-1", action: "created" },
    ]);
    // The destination really holds it, read back through the same binary.
    const copied = await copyClient.taskShow("into:T-1");
    expect(copied.items[0]?.item.metadata).toMatchObject({ "caller.count": 3 });

    const again = await copyClient.taskCopy(["from:T-1"], "into");
    expect(again.items[0]?.action).toBe("unchanged");

    // A destination with no write side is refused, naming the source and its plugin.
    writeFileSync(
      resolve(copyRoot, "onetaskgraph.yaml"),
      JSON.stringify({
        sources: {
          from: folder("from"),
          into: folder("into"),
          sealed: { plugin: "in-memory", config: { capabilities: { writes: "unsupported" } } },
        },
      }),
    );
    expect(readFileSync(resolve(copyRoot, "into/tasks/T-1.md"), "utf8")).toContain(
      "onetaskgraph.origin: from:T-1",
    );
    await expect(copyClient.taskCopy(["from:T-1"], "sealed")).rejects.toThrow("cannot be written");
  } finally {
    rmSync(copyRoot, { recursive: true, force: true });
  }
});

test("a source failure remains typed for partial and accepted-partial exits", async () => {
  const failureRoot = mkdtempSync(resolve(tmpdir(), "onetaskgraph-sdk-failure-"));
  writeFileSync(
    resolve(failureRoot, "onetaskgraph.yaml"),
    JSON.stringify({
      sources: {
        work: { plugin: "in-memory", config: { tasks: [] } },
        broken: { plugin: "linear", config: { endpoint: "://invalid" } },
      },
    }),
  );
  try {
    const failureClient = new OnetaskgraphClient({ binaryPath: binary, cwd: failureRoot });
    const partial = await failureClient.taskList({ sources: ["work", "broken"] });
    expect(partial.errors[0]?.source).toBe("broken");
    expect(partial.errors[0]?.error.kind).toBe("config");
    const accepted = await failureClient.taskList({
      sources: ["work", "broken"],
      allowPartial: true,
    });
    expect(accepted.errors[0]?.error.kind).toBe("config");
  } finally {
    rmSync(failureRoot, { recursive: true, force: true });
  }
});

test("explicit binary path takes precedence over the environment", () => {
  const resolved = new OnetaskgraphClient({
    binaryPath: binary,
    env: { ONETASKGRAPH_BIN: resolve(root, "wrong") },
  });
  expect(resolved.binaryPath).toBe(binary);
});

test("empty explicit and environment binary paths are rejected", () => {
  expect(() => new OnetaskgraphClient({ binaryPath: "" })).toThrow("non-empty executable path");
  expect(() => new OnetaskgraphClient({ env: { ONETASKGRAPH_BIN: " " } })).toThrow(
    "non-empty executable path",
  );
});

test("the supplied environment precedes packaged resolution", () => {
  const fromEnvironment = resolve(root, "from-environment");
  expect(new OnetaskgraphClient({ env: { ONETASKGRAPH_BIN: fromEnvironment } }).binaryPath).toBe(
    fromEnvironment,
  );
});

test("the process environment resolves and drives the real binary", async () => {
  const previous = process.env.ONETASKGRAPH_BIN;
  process.env.ONETASKGRAPH_BIN = binary;
  try {
    const processEnvironmentClient = new OnetaskgraphClient({ cwd: root });
    expect((await processEnvironmentClient.taskList({ sources: ["work"] })).items[0]?.id).toBe(
      "work:T-1",
    );
  } finally {
    if (previous === undefined) delete process.env.ONETASKGRAPH_BIN;
    else process.env.ONETASKGRAPH_BIN = previous;
  }
});

test("the PATH fallback drives the real binary", async () => {
  const pathClient = new OnetaskgraphClient({
    cwd: root,
    env: { PATH: [resolve(binary, ".."), "/usr/bin", "/bin"].join(delimiter) },
  });
  expect(pathClient.binaryPath).toBe("onetaskgraph");
  expect((await pathClient.taskList({ sources: ["work"] })).items[0]?.id).toBe("work:T-1");
});

test("a real command failure is a typed execution error", async () => {
  try {
    await client.taskShow("work:absent");
    throw new Error("the absent task unexpectedly succeeded");
  } catch (error) {
    expect(error).toBeInstanceOf(OnetaskgraphExecutionError);
    expect(errorMessage(error)).toContain("next:");
  }
});

test("an unavailable explicit executable is a typed execution error", async () => {
  const unavailable = new OnetaskgraphClient({ binaryPath: resolve(root, "missing-binary") });
  await expect(unavailable.taskList()).rejects.toBeInstanceOf(OnetaskgraphExecutionError);
});

test("non-JSON and malformed command output are rejected at the executable boundary", async () => {
  const fixtures = mkdtempSync(resolve(tmpdir(), "onetaskgraph-sdk-output-"));
  try {
    const nonJson = new OnetaskgraphClient({
      binaryPath: executableFixture(fixtures, "non-json", "not JSON"),
    });
    await expect(nonJson.taskList()).rejects.toBeInstanceOf(OnetaskgraphValidationError);

    const malformed = new OnetaskgraphClient({
      binaryPath: executableFixture(fixtures, "malformed", JSON.stringify({ items: [] })),
    });
    await expect(malformed.taskList()).rejects.toBeInstanceOf(OnetaskgraphValidationError);
  } finally {
    rmSync(fixtures, { recursive: true, force: true });
  }
});

test("schema output and unsupported exit statuses are validated from real executables", async () => {
  const fixtures = mkdtempSync(resolve(tmpdir(), "onetaskgraph-sdk-schema-output-"));
  try {
    const invalidBundle = new OnetaskgraphClient({
      binaryPath: executableFixture(fixtures, "invalid-bundle", JSON.stringify({ version: 1 })),
    });
    await expect(invalidBundle.schema()).rejects.toBeInstanceOf(OnetaskgraphValidationError);

    const validResponse = JSON.stringify(await client.taskList({ sources: ["work"] }));
    const wrongExit = new OnetaskgraphClient({
      binaryPath: executableFixture(fixtures, "wrong-exit", validResponse, "next: retry later", 9),
    });
    try {
      await wrongExit.taskList();
      throw new Error("the unsupported exit status unexpectedly succeeded");
    } catch (error) {
      expect(error).toBeInstanceOf(OnetaskgraphExecutionError);
      expect(errorMessage(error)).toContain("next: retry later");
      expect(errorMessage(error)).not.toContain("without a valid response");
    }

    const configResponse = JSON.stringify(await client.configShow());
    const partialConfig = new OnetaskgraphClient({
      binaryPath: executableFixture(fixtures, "partial-config", configResponse, "", 4),
    });
    await expect(partialConfig.configShow()).rejects.toBeInstanceOf(OnetaskgraphExecutionError);
  } finally {
    rmSync(fixtures, { recursive: true, force: true });
  }
});
