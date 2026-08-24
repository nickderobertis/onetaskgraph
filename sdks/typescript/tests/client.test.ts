import { afterAll, beforeAll, expect, test } from "bun:test";
import { chmodSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
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
  const path = resolve(directory, `${name}${windows ? ".cmd" : ""}`);
  const quote = (value: string) => JSON.stringify(value);
  const body = windows
    ? `@echo off\r\nnode -e "process.stdout.write(${quote(quote(stdout))}); process.stderr.write(${quote(quote(stderr))}); process.exit(${code})"\r\n`
    : `#!/usr/bin/env node\nprocess.stdout.write(${quote(stdout)});\nprocess.stderr.write(${quote(stderr)});\nprocess.exit(${code});\n`;
  writeFileSync(path, body);
  if (!windows) chmodSync(path, 0o755);
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
  if (tasks.next) {
    expect((await client.taskList({ sources: ["work"], page: tasks.next })).items).toBeArray();
  }
  expect((await client.taskList({ sources: ["work"], noProject: true })).items[0]?.id).toBe(
    "work:T-2",
  );
  expect((await client.taskShow("work:T-1", { allowPartial: true })).items[0]?.item.title).toBe(
    "Alpha engine",
  );
  expect((await client.taskDeps("work:T-2", { direction: "depended-on-by" })).items[0]?.from).toBe(
    "work:T-1",
  );
  expect((await client.projectList({ sources: ["work"] })).items[0]?.id).toBe("work:P-1");
  expect((await client.projectShow("work:P-1", { allowPartial: true })).items[0]?.item.title).toBe(
    "Engine",
  );
  expect(
    (await client.projectDeps("work:P-2", { direction: "depended-on-by" })).items[0]?.from,
  ).toBe("work:P-1");
  expect((await client.labelList({ sources: ["work"] })).items[0]?.id).toBe("work:L-1");
  expect(
    (await client.search("Alpha", { sources: ["work"], fields: "title", kind: "task", limit: 1 }))
      .items[0]?.kind,
  ).toBe("task");
});

test("a source failure remains typed for partial and accepted-partial exits", async () => {
  const failureRoot = mkdtempSync(resolve(tmpdir(), "onetaskgraph-sdk-failure-"));
  writeFileSync(
    resolve(failureRoot, "onetaskgraph.yaml"),
    JSON.stringify({
      sources: {
        work: { plugin: "in-memory", config: { tasks: [] } },
        broken: { plugin: "linear", config: {} },
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

test("the supplied environment precedes packaged resolution", () => {
  const fromEnvironment = resolve(root, "from-environment");
  expect(new OnetaskgraphClient({ env: { ONETASKGRAPH_BIN: fromEnvironment } }).binaryPath).toBe(
    fromEnvironment,
  );
});

test("the PATH fallback drives the real binary", async () => {
  const pathClient = new OnetaskgraphClient({
    cwd: root,
    env: { PATH: `${resolve(binary, "..")}:/usr/bin:/bin` },
  });
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
  expect(unavailable.taskList()).rejects.toBeInstanceOf(OnetaskgraphExecutionError);
});

test("non-JSON and malformed command output are rejected at the executable boundary", async () => {
  const fixtures = mkdtempSync(resolve(tmpdir(), "onetaskgraph-sdk-output-"));
  try {
    const nonJson = new OnetaskgraphClient({
      binaryPath: executableFixture(fixtures, "non-json", "not JSON"),
    });
    expect(nonJson.taskList()).rejects.toBeInstanceOf(OnetaskgraphValidationError);

    const malformed = new OnetaskgraphClient({
      binaryPath: executableFixture(fixtures, "malformed", JSON.stringify({ items: [] })),
    });
    expect(malformed.taskList()).rejects.toBeInstanceOf(OnetaskgraphValidationError);
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
    expect(invalidBundle.schema()).rejects.toBeInstanceOf(OnetaskgraphValidationError);

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
  } finally {
    rmSync(fixtures, { recursive: true, force: true });
  }
});
