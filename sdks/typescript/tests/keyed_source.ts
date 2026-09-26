// A read-only task source whose backend shows people a short handle, over the stdio protocol.
//
// The SDK tests need a source that reports a task's `key`, and none of the credential-free
// sources this build ships can: `local-md` and `in-memory` have no handle of their own by
// contract, and the two backends that do — Linear and GitHub — need a credential. So this
// peer, spoken to over docs/plugin-protocol.md exactly as the engine speaks to any plugin,
// holds two tasks: one whose backend gave it a handle, and one sent the way a plugin written
// before `key` existed sends a task, with no `key` member at all (§4.13a).
import { createInterface } from "node:readline";

const PROTOCOL_VERSION = 2;

const unfiled = {
  content: null,
  status: { category: "todo", name: "Todo" },
  labels: [],
  project: null,
  url: null,
  location: null,
  created_at: null,
  updated_at: null,
};

const tasks = [
  { id: "iss_8f2c", key: "ENG-7", title: "Engine handle", ...unfiled },
  { id: "iss_91d0", title: "Engine without a handle", ...unfiled },
];

// Everything a query could narrow by is declared unsupported, so this peer answers every
// `query_tasks` with the whole set and the engine applies the predicates itself (rule 2).
const capabilities = {
  projects: "unsupported",
  documents: "unsupported",
  orphan_tasks: "native",
  filter_by_label: "unsupported",
  filter_by_status: "unsupported",
  search_title: "unsupported",
  search_content: "unsupported",
  task_dependencies: "forward-only",
  project_dependencies: "forward-only",
  max_page_size: 50,
};

type Request = { id: string; method: string; params: { id?: unknown } };

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
  return { id, method, params };
}

function answer(request: Request): object {
  switch (request.method) {
    case "initialize":
      return {
        result: {
          protocol_version: PROTOCOL_VERSION,
          kind: "keyed-source",
          capabilities,
          writes: "unsupported",
        },
      };
    case "health":
      return { result: { reachable: true, detail: `${tasks.length} task(s)` } };
    case "get_task":
      return { result: { task: tasks.find((task) => task.id === request.params.id) ?? null } };
    case "query_tasks":
      return { result: { items: tasks, next: null } };
    case "labels":
    case "task_dependencies":
      return { result: { items: [], next: null } };
    default:
      return {
        error: {
          kind: "malformed",
          message: `protocol version ${PROTOCOL_VERSION} has no method ${request.method} this source serves`,
        },
      };
  }
}

for await (const line of createInterface({ input: process.stdin })) {
  if (line.trim() === "") continue;
  const request = requestOf(line);
  if (request === undefined) {
    process.stderr.write("keyed-source: ignoring a line that is not a request\n");
    continue;
  }
  process.stdout.write(`${JSON.stringify({ id: request.id, ...answer(request) })}\n`);
}
