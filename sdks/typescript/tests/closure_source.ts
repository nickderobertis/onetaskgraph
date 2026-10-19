// A board-like source answering both closure methods, over the stdio protocol.
//
// The SDK tests need a source that answers `task status reconcile` and `task status complete`
// with items, and none of the credential-free sources this build ships can: `local-md` and
// `in-memory` read their status as their closure and declare neither, and the one that does —
// GitHub Projects — needs a credential and a board. So this peer, spoken to over
// docs/plugin-protocol.md exactly as the engine speaks to any plugin, holds a plan's home
// project `home` and a task `t1`, and answers each closure method with both: the home as a
// project item, the task as a task item (§4.23, §4.24).
import { createInterface } from "node:readline";

const PROTOCOL_VERSION = 2;

// What a write of each terminal category reads back as, as a board's options spell them.
const NAMES: Record<string, string> = { done: "Done", cancelled: "Cancelled" };

const unwritten = { content: null, labels: [], url: null, created_at: null, updated_at: null };
const home = {
  id: "home",
  title: "A plan's home",
  status: { category: "in-progress", name: "In Progress" },
  ...unwritten,
};
const task = {
  id: "t1",
  title: "A task of the plan",
  status: { category: "todo", name: "Todo" },
  project: "home",
  ...unwritten,
};

const capabilities = {
  projects: "native",
  documents: "unsupported",
  orphan_tasks: "native",
  filter_by_label: "unsupported",
  filter_by_status: "unsupported",
  search_title: "unsupported",
  search_content: "unsupported",
  task_dependencies: "forward-only",
  project_dependencies: "forward-only",
  max_page_size: 50,
  reconcile_status: "native",
  complete_by_issue: "native",
};

// The issue each item is held for: `acme/plan#1` is the home's, `acme/plan#2` the task's.
const holdings: Record<string, object> = {
  "acme/plan#1": { id: "home", kind: "project", option: "In Progress" },
  "acme/plan#2": { id: "t1", option: "Todo" },
};

type Request = { id: string; method: string; params: Record<string, unknown> };

// The `id`, `method` and `params` of one request line (§2). A line without a string `id` has
// no address a response could echo, so the caller sets it aside rather than answering it.
function requestOf(line: string): Request | undefined {
  let value: unknown;
  try {
    value = JSON.parse(line);
  } catch {
    return undefined;
  }
  if (typeof value !== "object" || value === null) return undefined;
  if (!("id" in value && "method" in value && "params" in value)) return undefined;
  const { id, method, params } = value;
  if (typeof id !== "string" || typeof method !== "string") return undefined;
  if (typeof params !== "object" || params === null || Array.isArray(params)) return undefined;
  return { id, method, params: params as Record<string, unknown> };
}

function malformed(message: string): object {
  return { error: { kind: "malformed", message } };
}

// A status write of the one item of its kind this source holds, read back as written.
function statusWritten(params: Record<string, unknown>, held: string): object {
  const category = params.category;
  if (typeof category !== "string") return malformed("a status write names its category");
  if (params.id !== held) return { result: { status: null } };
  return { result: { status: { category, name: NAMES[category] ?? category } } };
}

function answer(request: Request): object {
  switch (request.method) {
    case "initialize":
      return {
        result: {
          protocol_version: PROTOCOL_VERSION,
          kind: "closure-source",
          capabilities,
          writes: "supported",
          task_updates: true,
          project_status: true,
        },
      };
    case "health":
      return { result: { reachable: true, detail: "one project and one task" } };
    case "status_contradictions":
      return {
        result: {
          items: [
            { project: home, option: "In Progress", closed_as: "done", to: "Done" },
            { task, option: "Todo", closed_as: "cancelled", to: "Cancelled" },
          ],
        },
      };
    case "issue_holdings": {
      const issues = request.params.issues;
      if (!Array.isArray(issues)) return malformed("issue_holdings names its issues as a list");
      return {
        result: {
          holdings: issues.map((issue) => {
            const held = typeof issue === "string" ? holdings[issue] : undefined;
            return held === undefined
              ? { issue, completion: "absent" }
              : { issue, completion: "due", task: held, to: "Done" };
          }),
        },
      };
    }
    case "set_task_status":
      return statusWritten(request.params, "t1");
    case "set_project_status":
      return statusWritten(request.params, "home");
    default:
      return malformed(
        `protocol version ${PROTOCOL_VERSION} has no method ${request.method} this source serves`,
      );
  }
}

for await (const line of createInterface({ input: process.stdin })) {
  if (line.trim() === "") continue;
  const request = requestOf(line);
  if (request === undefined) {
    process.stderr.write("closure-source: ignoring a line that is not a request\n");
    continue;
  }
  process.stdout.write(`${JSON.stringify({ id: request.id, ...answer(request) })}\n`);
}
