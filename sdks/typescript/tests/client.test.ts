import { afterAll, beforeAll, expect, test } from "bun:test";
import {
  appendFileSync,
  chmodSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
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
            // A source holding documents has to declare it: the engine reads the
            // declaration once at the handshake and never asks a source that says it has
            // none, so a `documents:` list without this key is refused where it is read.
            capabilities: { documents: "native" },
            documents: [
              {
                id: "D-1",
                title: "Alpha design",
                content: "the engine core, reviewed",
                project: "P-1",
                labels: [{ id: "L-1", name: "bug" }],
                location: { url: "https://example.invalid/D-1" },
              },
              { id: "D-2", title: "Loose note", content: "filed nowhere", labels: [] },
            ],
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

// This file's first command is the first time anything here starts the debug binary, and
// that first exec is not the same cost as the ones after it: on a loaded macOS runner,
// paging in an unstripped debug build and validating its signature has outlasted bun's 5s
// default on its own, while the test below drove a dozen commands through the warm binary
// in under two seconds. A bound this wide still catches a command that hangs; what it stops
// doing is reporting a cold start as one.
const COLD_START_TIMEOUT_MS = 60_000;

test(
  "every emitted command has a client method",
  async () => {
    assertCompleteCommandSurface();
    const bundle = await client.schema();
    expect(bundle).toHaveProperty("commands", [...clientCommands]);
  },
  COLD_START_TIMEOUT_MS,
);

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
  const documents = await client.documentList({
    sources: ["work"],
    labels: ["bug"],
    excludeLabels: ["chore"],
    search: "Alpha",
    fields: "title",
    limit: 1,
    project: "P-1",
  });
  expect(documents.items[0]?.id).toBe("work:D-1");
  // Where a document is comes back as the contract type's own JSON, so a caller branches
  // on which key is present rather than parsing a sentence.
  expect(documents.items[0]?.item.location).toEqual({ url: "https://example.invalid/D-1" });
  expect((await client.documentList({ sources: ["work"], noProject: true })).items[0]?.id).toBe(
    "work:D-2",
  );
  expect((await client.documentShow("work:D-1", { allowPartial: true })).items[0]?.item.title).toBe(
    "Alpha design",
  );
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
  mkdirSync(resolve(copyRoot, "from/projects"), { recursive: true });
  mkdirSync(resolve(copyRoot, "into"), { recursive: true });
  writeFileSync(
    resolve(copyRoot, "from/projects/P-1.md"),
    "---\ntitle: Engine\nstatus: todo\n---\nthe engine\n",
  );
  writeFileSync(
    resolve(copyRoot, "from/tasks/T-1.md"),
    "---\ntitle: Alpha engine\nstatus: todo\nproject: P-1\nmetadata: {caller.count: 3}\n---\nthe engine core\n",
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
    // Null only for a dry run that would create: nothing was, so there is no id.
    expect(planned.items).toEqual([{ source: "from:T-1", action: "created", destination: null }]);

    const created = await copyClient.taskCopy(["from:T-1"], "into");
    expect(created.items).toEqual([
      { source: "from:T-1", action: "created", destination: "into:T-1" },
    ]);
    // The destination really holds it, read back through the same binary.
    const copied = await copyClient.taskShow("into:T-1");
    expect(copied.items[0]?.item.metadata).toMatchObject({ "caller.count": 3 });
    expect(readFileSync(resolve(copyRoot, "into/tasks/T-1.md"), "utf8")).toContain(
      "onetaskgraph.origin: from:T-1",
    );

    const again = await copyClient.taskCopy(["from:T-1"], "into");
    expect(again.items[0]?.action).toBe("unchanged");

    // A person deletes the origin key, so neither rule can find the counterpart; the
    // caller-named escape re-establishes it rather than creating a second item.
    writeFileSync(
      resolve(copyRoot, "into/tasks/T-1.md"),
      readFileSync(resolve(copyRoot, "into/tasks/T-1.md"), "utf8").replace(
        /\n\s+onetaskgraph.origin: from:T-1/,
        "",
      ),
    );
    const matched = await copyClient.taskCopy(["from:T-1"], "into", { matchBy: "title" });
    expect(matched.items).toEqual([
      { source: "from:T-1", action: "updated", destination: "into:T-1" },
    ]);

    // An origin naming nothing at the destination refuses, and --recreate says to create.
    rmSync(resolve(copyRoot, "from/tasks/T-1.md"));
    await expect(copyClient.taskCopy(["into:T-1"], "from")).rejects.toThrow(
      "which that destination no longer holds",
    );
    const recreated = await copyClient.taskCopy(["into:T-1"], "from", { recreate: true });
    expect(recreated.items[0]?.action).toBe("created");

    // A project, and the tasks in it, and then the project on its own.
    const project = await copyClient.projectCopy("from:P-1", "into");
    expect(project.items.map((item) => [item.source, item.action])).toEqual([
      ["from:P-1", "created"],
      // The task recreated above already corresponds to the one at the destination, so
      // copying the project it belongs to matches it rather than duplicating it.
      ["from:T-1", "unchanged"],
    ]);
    const alone = await copyClient.projectCopy("from:P-1", "into", { noTasks: true });
    expect(alone.items).toEqual([
      { source: "from:P-1", action: "unchanged", destination: "into:P-1" },
    ]);

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
    await expect(copyClient.taskCopy(["from:T-1"], "sealed")).rejects.toThrow("cannot be written");
  } finally {
    rmSync(copyRoot, { recursive: true, force: true });
  }
});

test("a document copy drives the real binary and is refused by a source with none", async () => {
  // `notes` is a folder of Markdown, whose documents are files that outlive the process,
  // so what one client call copied the next one reads back — which is what proves the copy
  // landed rather than only that a report was printed. `sealed` is an in-memory source
  // without the `documents` capability, so it holds none and is refused as a destination
  // before anything is read.
  const documentRoot = mkdtempSync(resolve(tmpdir(), "onetaskgraph-sdk-document-"));
  mkdirSync(resolve(documentRoot, "notes"), { recursive: true });
  writeFileSync(
    resolve(documentRoot, "onetaskgraph.yaml"),
    JSON.stringify({
      sources: {
        from: {
          plugin: "in-memory",
          config: {
            capabilities: { documents: "native" },
            documents: [
              {
                id: "D-1",
                title: "Alpha design",
                content: "reviewed",
                labels: [],
                location: { url: "https://example.invalid/D-1" },
                metadata: { "caller.reviewers": ["ada", "grace"], "caller.rounds": 2 },
              },
            ],
          },
        },
        notes: {
          plugin: "local-md",
          config: { root: resolve(documentRoot, "notes"), status_mapping: { todo: "todo" } },
        },
        sealed: { plugin: "in-memory", config: {} },
      },
    }),
  );
  try {
    const documentClient = new OnetaskgraphClient({ binaryPath: binary, cwd: documentRoot });

    const planned = await documentClient.documentCopy(["from:D-1"], "notes", { dryRun: true });
    expect(planned.items).toEqual([{ source: "from:D-1", action: "created", destination: null }]);

    const created = await documentClient.documentCopy(["from:D-1"], "notes");
    expect(created.items).toEqual([
      { source: "from:D-1", action: "created", destination: "notes:D-1" },
    ]);

    // The destination really holds it, read back through the same binary: every
    // caller-defined key with its JSON type intact, and a location that is the
    // destination's own — the path of the file this folder put it in, not the URL the
    // source reported.
    const copied = await documentClient.documentShow("notes:D-1");
    const document = copied.items[0]?.item;
    // The location is compared by the file it names rather than by how it is spelled: this
    // source canonicalizes, and a canonical path is spelled differently on each platform —
    // macOS resolves the temporary tree's symlink under `/var/folders`, and Windows answers
    // with an extended-length `\\?\` path over an account name no other language here
    // writes. Resolving both sides through `fs.realpathSync` does not settle that: on
    // Windows this runtime returns each path unchanged, so the comparison stayed
    // `\\?\C:\Users\runneradmin\…` against the `C:\Users\RUNNER~1\…` short form this test
    // built, which is what `check (windows-latest)` refused. Write a sentinel through the
    // path this test built and read it back through the path the source reported instead:
    // one file has it and no other file can, which is the question the assertion is really
    // making, and reading also fails outright when the reported path names nothing — as
    // comparing two strings does not. The extended-length prefix is dropped first, because
    // it is a spelling for Windows' own API rather than for a file call of this runtime —
    // and only ahead of a drive letter, the one form a temporary tree takes, so the `UNC\\`
    // spelling this test never produces is left whole rather than turned into a bad path.
    const located = document?.location as { path: string };
    const openable = located.path.replace(/^\\\\\?\\(?=[A-Za-z]:\\)/, "");
    const sentinel = "read back through the location this source reported";
    appendFileSync(resolve(documentRoot, "notes/documents/D-1.md"), `\n${sentinel}\n`);
    expect(readFileSync(openable, "utf8")).toContain(sentinel);
    expect(document).toMatchObject({
      title: "Alpha design",
      content: "reviewed",
      url: null,
      metadata: {
        "caller.reviewers": ["ada", "grace"],
        "caller.rounds": 2,
        "onetaskgraph.origin": "from:D-1",
      },
    });

    await expect(documentClient.documentCopy(["from:D-1"], "sealed")).rejects.toThrow(
      "has no documents",
    );
  } finally {
    rmSync(documentRoot, { recursive: true, force: true });
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
