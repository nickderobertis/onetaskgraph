import "./ambient.ts";
import { expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import {
  type CopyReport,
  OnetaskgraphClient,
  OnetaskgraphExecutionError,
  projectCopyFlags,
} from "../src/index.ts";

// The plan-review and copy-back additions, through the SDK against the real binary:
// `projectAttestationCheck` with `tasks` narrowed by `statuses`, `notStatuses` and `states`,
// `taskList`'s `notStatuses`, `taskDigest`, and `projectCopy`'s `updateFields` and `metadata`,
// each answer validated against the root the binary emits for it.

const binary = resolve(import.meta.dir, "../../../target/debug/onetaskgraph");

function store(prefix: string, sources: Record<string, unknown>, extra: object = {}) {
  const root = mkdtempSync(resolve(tmpdir(), prefix));
  for (const name of Object.keys(sources)) mkdirSync(resolve(root, name));
  writeFileSync(resolve(root, "onetaskgraph.yaml"), JSON.stringify({ sources, ...extra }));
  // The store's home is the scratch tree, so a user configuration on the host adds no source.
  const client = new OnetaskgraphClient({
    binaryPath: binary,
    cwd: root,
    env: { ...process.env, HOME: root, XDG_CONFIG_HOME: root },
  });
  return { root, client };
}

function created(report: CopyReport, source: string): string {
  const outcome = report.items.find((item) => item.source === source);
  if (outcome === undefined || !("destination" in outcome) || !outcome.destination) {
    throw new Error(`no destination for ${source}`);
  }
  return outcome.destination;
}

test("a review's open tasks, a not-status list and a task's digest", async () => {
  const { root, client } = store("onetaskgraph-review-", {
    notes: { plugin: "local-md", config: { root: "notes" } },
  });
  try {
    await client.projectCreate("notes", "P-1", "Plan", { body: "The plan.\n" });
    const ids: Record<string, string> = {};
    for (const [title, status] of [
      ["open", "todo"],
      ["reviewed", "todo"],
      ["shipped", "done"],
    ] as const) {
      const made = await client.taskCreate("notes", "P-1", title, { body: `${title}\n`, status });
      ids[title] = made.items[0]?.id ?? "";
    }
    const attested = await client.taskAttest(ids.reviewed ?? "", "plan-review", "ada", "bar-1", {
      exclude: ["myapp.x"],
    });

    const owed = await client.projectAttestationCheck("notes:P-1", "plan-review", {
      tasks: true,
      notStatuses: ["done", "cancelled"],
      states: ["void", "absent"],
    });
    expect("tasks" in owed && owed.tasks.map((entry: { id: string }) => entry.id)).toEqual([
      ids.open,
    ]);
    const valid = await client.projectAttestationCheck("notes:P-1", "plan-review", {
      tasks: true,
      statuses: ["todo"],
      states: ["valid"],
    });
    expect("tasks" in valid && valid.tasks.map((entry: { id: string }) => entry.id)).toEqual([
      ids.reviewed,
    ]);

    const listed = await client.taskList({ project: "notes:P-1", notStatuses: ["done"] });
    expect(listed.items.map((task) => task.id).sort()).toEqual(
      [ids.open ?? "", ids.reviewed ?? ""].sort(),
    );

    const digest = await client.taskDigest(ids.reviewed ?? "", { exclude: ["myapp.x"] });
    expect("record" in attested && digest.content_digest).toBe(
      "record" in attested ? attested.record.content_digest : "",
    );
    await expect(client.taskDigest("notes:missing")).rejects.toThrow(OnetaskgraphExecutionError);
    // Untyped JavaScript callers can pass options that are not an object, or keys that are not
    // strings; each is refused before the binary is spawned.
    expect(() =>
      client.taskDigest(ids.reviewed ?? "", "myapp.x" as unknown as { exclude?: string[] }),
    ).toThrow("options is not an object");
    expect(() =>
      client.taskDigest(ids.reviewed ?? "", { exclude: [1] as unknown as string[] }),
    ).toThrow("exclude");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("a board's copy of a private plan copies back in place, its status and keys alone", async () => {
  const passing = 'import sys; sys.stdin.read(); print(\'{"verdict": "pass"}\')';
  const publicRepo = 'import sys; sys.stdin.read(); print(\'{"visibility": "public"}\')';
  const { root, client } = store(
    "onetaskgraph-copy-back-",
    {
      authoring: { plugin: "local-md", visibility: "private", config: { root: "authoring" } },
      board: { plugin: "local-md", visibility: "public", config: { root: "board" } },
    },
    {
      write_policy: {
        check_command: ["python3", "-c", passing],
        visibility_command: ["python3", "-c", publicRepo],
      },
    },
  );
  try {
    await client.projectCreate("authoring", "plan-1", "Plan", { body: "The plan.\n" });
    const made = await client.taskCreate("authoring", "plan-1", "Build", { body: "Build it.\n" });
    const authored = made.items[0]?.id ?? "";
    const onto = await client.projectCopy("authoring:plan-1", "board");
    const boardProject = created(onto, "authoring:plan-1");
    const boardTask = created(onto, authored);
    await client.taskStatusSet(boardTask, "done");

    const back = await client.projectCopy(boardProject, "authoring", {
      updateFields: "status",
      metadata: ['host.m={"a":1}'],
    });
    expect(
      "items" in back && back.items.map((item: { action: string }) => item.action).sort(),
    ).toEqual(["updated", "updated"]);
    const shown = (await client.taskShow(authored, { noComments: true })).items[0]?.item;
    expect([shown?.status.category, shown?.content, shown?.metadata?.["host.m"]]).toEqual([
      "done",
      "Build it.\n",
      { a: 1 },
    ]);

    // A runtime caller can arrive without TypeScript's union check.
    expect(() => projectCopyFlags({ updateFields: "title" as "status" })).toThrow("updateFields");
    expect(projectCopyFlags({ updateFields: "status", metadata: ["k=null"] })).toEqual([
      "--metadata",
      "k=null",
      "--update-fields",
      "status",
    ]);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
