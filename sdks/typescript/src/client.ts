import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import Ajv2020 from "ajv/dist/2020.js";
import { binaryCommands } from "./generated/commands.ts";
import type {
  Comment,
  CommentList,
  CopyReport,
  DeletedComment,
  EffectiveConfig,
  FieldsReport,
  MetadataSet,
  Priority,
  QueryResponseOfQualifiedDocument,
  QueryResponseOfQualifiedEdge,
  QueryResponseOfQualifiedLabel,
  QueryResponseOfQualifiedProject,
  QueryResponseOfQualifiedTask,
  QueryResponseOfSearchHit,
  SourceListings,
  StatusCategory,
  StatusOptionsReport,
  TaskContentSet,
  TaskDetail,
  TaskPrioritySet,
  TaskStatusSet,
} from "./generated/models.ts";
import { runtimeSchemas } from "./generated/schemas.ts";
import { SCHEMA_BUNDLE_VERSION } from "./generated/models.ts";

export type QueryOptions = {
  sources?: string[];
  limit?: number;
  page?: string;
  allowPartial?: boolean;
};
export type DependencyOptions = Omit<QueryOptions, "sources"> & {
  direction?: "depends-on" | "depended-on-by";
};
export type FilterOptions = QueryOptions & {
  labels?: string[];
  excludeLabels?: string[];
  statuses?: string[];
  search?: string;
  fields?: "title" | "content" | "both";
};
// A document has no status, so a document list has no status filter — and this type is
// what stops a caller writing one down for a verb the binary would refuse it on.
export type DocumentFilterOptions = Omit<FilterOptions, "statuses">;
export type CopyOptions = {
  matchBy?: string;
  recreate?: boolean;
  dryRun?: boolean;
};
// A comment's body, given as text the client writes to the binary's standard input or as a
// file the binary reads byte for byte — never as a word of the command line.
export type CommentBodyOptions = { body: string } | { bodyFile: string };
export type CommentAddOptions = CommentBodyOptions & { author?: string };
export type ClientOptions = {
  binaryPath?: string;
  cwd?: string;
  env?: NodeJS.ProcessEnv;
};

export class OnetaskgraphExecutionError extends Error {
  constructor(
    readonly exitCode: number | null,
    readonly stderr = "",
  ) {
    const diagnostic = stderr.trim();
    super(
      `onetaskgraph execution failed (exit ${exitCode ?? "signal"})` +
        (diagnostic.length > 0 ? `: ${diagnostic}` : ""),
    );
    this.name = "OnetaskgraphExecutionError";
  }
}

export class OnetaskgraphValidationError extends Error {
  constructor(
    readonly command: string,
    readonly validationErrors: unknown,
  ) {
    super(`onetaskgraph emitted an invalid response for '${command}'`);
    this.name = "OnetaskgraphValidationError";
  }
}

const responseRoots: Record<string, keyof typeof runtimeSchemas> = {
  "config show": "EffectiveConfig",
  "sources list": "SourceListings",
  "sources status-options": "StatusOptionsReport",
  "sources fields": "FieldsReport",
  "task list": "QueryResponseOfQualifiedTask",
  "task show": "TaskDetail",
  "task deps": "QueryResponseOfQualifiedEdge",
  "task copy": "CopyReport",
  "task comment add": "Comment",
  "task comment list": "CommentList",
  "task comment edit": "Comment",
  "task comment delete": "DeletedComment",
  "task status set": "TaskStatusSet",
  "task priority set": "TaskPrioritySet",
  "task content set": "TaskContentSet",
  "task metadata set": "MetadataSet",
  "project list": "QueryResponseOfQualifiedProject",
  "project show": "QueryResponseOfQualifiedProject",
  "project deps": "QueryResponseOfQualifiedEdge",
  "project copy": "CopyReport",
  "project metadata set": "MetadataSet",
  "document list": "QueryResponseOfQualifiedDocument",
  "document show": "QueryResponseOfQualifiedDocument",
  "document copy": "CopyReport",
  "document metadata set": "MetadataSet",
  "label list": "QueryResponseOfQualifiedLabel",
  search: "QueryResponseOfSearchHit",
};

// Exit 4 is a whole answer with part of it missing: a read some sources could not answer, or a
// write — a copy, or `task status set` — that landed and could not keep a task it delivers in
// step, which its response names. A comment verb is one call to one source and neither reads
// several nor keeps anything in step, so exit 4 is not a code it can produce and not one this
// client accepts from it. A `metadata set` is the same: one write to one source, and metadata is
// not status, so it keeps no delivered task in step — and so are `priority set` and `content
// set`, for the same reason.
const partialResponseCommands = new Set(
  Object.keys(responseRoots).filter(
    (command) =>
      command !== "config show" &&
      command !== "sources list" &&
      !command.startsWith("task comment ") &&
      !command.endsWith(" metadata set") &&
      !command.endsWith(" priority set") &&
      !command.endsWith(" content set"),
  ),
);

function bodyArguments(options: CommentBodyOptions): { args: string[]; input?: string } {
  return "bodyFile" in options
    ? { args: ["--body-file", options.bodyFile] }
    : { args: [], input: options.body };
}

export const clientCommands: readonly string[] = binaryCommands;

function packagedBinary(): string {
  const require = createRequire(import.meta.url);
  try {
    const manifest = require.resolve("@onetaskgraph/cli/package.json");
    const suffix = process.platform === "win32" ? "onetaskgraph.exe" : "onetaskgraph";
    return resolve(dirname(manifest), "bin", suffix);
  } catch {
    return "onetaskgraph";
  }
}

function requireBinaryPath(binaryPath: string): string {
  if (binaryPath.trim() === "") {
    throw new TypeError("binaryPath must be a non-empty executable path");
  }
  return binaryPath;
}

function addQuery(args: string[], options: QueryOptions): void {
  for (const source of options.sources ?? []) args.push("--source", source);
  addPage(args, options);
}

function addPage(args: string[], options: Omit<QueryOptions, "sources">): void {
  if (options.limit !== undefined) args.push("--limit", String(options.limit));
  if (options.page !== undefined) args.push("--page", options.page);
  if (options.allowPartial) args.push("--allow-partial");
}

function addFilters(args: string[], options: FilterOptions): void {
  addQuery(args, options);
  for (const label of options.labels ?? []) args.push("--label", label);
  for (const label of options.excludeLabels ?? []) args.push("--not-label", label);
  for (const status of options.statuses ?? []) args.push("--status", status);
  if (options.search !== undefined) args.push("--search", options.search);
  if (options.fields !== undefined) args.push("--in", options.fields);
}

function copyFlags(options: CopyOptions): string[] {
  const args: string[] = [];
  if (options.matchBy !== undefined) args.push("--match-by", options.matchBy);
  if (options.recreate) args.push("--recreate");
  if (options.dryRun) args.push("--dry-run");
  return args;
}

export class OnetaskgraphClient {
  readonly binaryPath: string;
  readonly cwd: string | undefined;
  readonly env: NodeJS.ProcessEnv;

  constructor(options: ClientOptions = {}) {
    this.binaryPath = requireBinaryPath(
      options.binaryPath ??
        options.env?.ONETASKGRAPH_BIN ??
        process.env.ONETASKGRAPH_BIN ??
        packagedBinary(),
    );
    this.cwd = options.cwd;
    this.env = { ...process.env, ...options.env };
    delete this.env.ONETASKGRAPH_BIN;
  }

  schema(): Promise<unknown> {
    return this.run("schema", []);
  }
  configShow(): Promise<EffectiveConfig> {
    return this.run("config show", []);
  }
  sourcesList(): Promise<SourceListings> {
    return this.run("sources list", []);
  }
  sourcesStatusOptions(
    source: string,
    options: { apply?: boolean } = {},
  ): Promise<StatusOptionsReport> {
    return this.run("sources status-options", [source, ...(options.apply ? ["--apply"] : [])]);
  }
  sourcesFields(source: string, options: { apply?: boolean } = {}): Promise<FieldsReport> {
    return this.run("sources fields", [source, ...(options.apply ? ["--apply"] : [])]);
  }
  taskList(
    options: FilterOptions & {
      project?: string;
      noProject?: boolean;
      priorities?: Priority[];
    } = {},
  ): Promise<QueryResponseOfQualifiedTask> {
    const args: string[] = [];
    addFilters(args, options);
    if (options.project !== undefined) args.push("--project", options.project);
    if (options.noProject) args.push("--no-project");
    for (const priority of options.priorities ?? []) args.push("--priority", priority);
    return this.run("task list", args);
  }
  taskShow(id: string, options: Pick<QueryOptions, "allowPartial"> = {}): Promise<TaskDetail> {
    return this.run("task show", [id, ...(options.allowPartial ? ["--allow-partial"] : [])]);
  }
  taskCommentAdd(id: string, options: CommentAddOptions): Promise<Comment> {
    const { args, input } = bodyArguments(options);
    if (options.author !== undefined) args.push("--author", options.author);
    return this.run("task comment add", [id, ...args], input);
  }
  taskCommentList(id: string): Promise<CommentList> {
    return this.run("task comment list", [id]);
  }
  taskCommentEdit(id: string, commentId: string, options: CommentBodyOptions): Promise<Comment> {
    const { args, input } = bodyArguments(options);
    return this.run("task comment edit", [id, commentId, ...args], input);
  }
  taskCommentDelete(id: string, commentId: string): Promise<DeletedComment> {
    return this.run("task comment delete", [id, commentId]);
  }
  taskStatusSet(id: string, category: StatusCategory): Promise<TaskStatusSet> {
    return this.run("task status set", [id, category]);
  }
  taskPrioritySet(id: string, priority: Priority): Promise<TaskPrioritySet> {
    return this.run("task priority set", [id, priority]);
  }
  // `file` is read by the binary byte for byte, and its bytes replace the task's content.
  taskContentSet(id: string, file: string): Promise<TaskContentSet> {
    return this.run("task content set", [id, "--file", file]);
  }
  // `value` is the JSON text the binary parses strictly, exactly the word the command line takes.
  taskMetadataSet(id: string, key: string, value: string): Promise<MetadataSet> {
    return this.run("task metadata set", [id, key, value]);
  }
  taskDeps(id: string, options: DependencyOptions = {}): Promise<QueryResponseOfQualifiedEdge> {
    const args = [id];
    addPage(args, options);
    if (options.direction) args.push("--direction", options.direction);
    return this.run("task deps", args);
  }
  taskCopy(ids: string[], to: string, options: CopyOptions = {}): Promise<CopyReport> {
    return this.run("task copy", [...ids, "--to", to, ...copyFlags(options)]);
  }
  projectList(options: FilterOptions = {}): Promise<QueryResponseOfQualifiedProject> {
    const args: string[] = [];
    addFilters(args, options);
    return this.run("project list", args);
  }
  projectShow(
    id: string,
    options: Pick<QueryOptions, "allowPartial"> = {},
  ): Promise<QueryResponseOfQualifiedProject> {
    return this.run("project show", [id, ...(options.allowPartial ? ["--allow-partial"] : [])]);
  }
  projectDeps(id: string, options: DependencyOptions = {}): Promise<QueryResponseOfQualifiedEdge> {
    const args = [id];
    addPage(args, options);
    if (options.direction) args.push("--direction", options.direction);
    return this.run("project deps", args);
  }
  projectCopy(
    id: string,
    to: string,
    options: CopyOptions & { noTasks?: boolean; members?: string[] } = {},
  ): Promise<CopyReport> {
    const args = [id, "--to", to, ...copyFlags(options)];
    if (options.noTasks) args.push("--no-tasks");
    for (const member of options.members ?? []) args.push("--member", member);
    return this.run("project copy", args);
  }
  projectMetadataSet(id: string, key: string, value: string): Promise<MetadataSet> {
    return this.run("project metadata set", [id, key, value]);
  }
  documentList(
    options: DocumentFilterOptions & { project?: string; noProject?: boolean } = {},
  ): Promise<QueryResponseOfQualifiedDocument> {
    const args: string[] = [];
    addFilters(args, options);
    if (options.project !== undefined) args.push("--project", options.project);
    if (options.noProject) args.push("--no-project");
    return this.run("document list", args);
  }
  documentShow(
    id: string,
    options: Pick<QueryOptions, "allowPartial"> = {},
  ): Promise<QueryResponseOfQualifiedDocument> {
    return this.run("document show", [id, ...(options.allowPartial ? ["--allow-partial"] : [])]);
  }
  documentCopy(ids: string[], to: string, options: CopyOptions = {}): Promise<CopyReport> {
    return this.run("document copy", [...ids, "--to", to, ...copyFlags(options)]);
  }
  documentMetadataSet(id: string, key: string, value: string): Promise<MetadataSet> {
    return this.run("document metadata set", [id, key, value]);
  }
  labelList(options: QueryOptions = {}): Promise<QueryResponseOfQualifiedLabel> {
    const args: string[] = [];
    addQuery(args, options);
    return this.run("label list", args);
  }
  search(
    text: string,
    options: QueryOptions & {
      fields?: "title" | "content" | "both";
      kind?: "task" | "project" | "both";
    } = {},
  ): Promise<QueryResponseOfSearchHit> {
    const args = [text];
    addQuery(args, options);
    if (options.fields) args.push("--in", options.fields);
    if (options.kind) args.push("--kind", options.kind);
    return this.run("search", args);
  }

  private run<T>(command: string, args: string[], input?: string): Promise<T> {
    const commandArgs = [...command.split(" "), ...args, "--json"];
    return new Promise((resolvePromise, reject) => {
      // Standard input is always a pipe, written with the body when there is one and closed
      // at once either way: a command that reads a body there must never wait on this
      // process's own input, and one given none reads an empty body and says so.
      const child = spawn(this.binaryPath, commandArgs, {
        cwd: this.cwd,
        env: this.env,
        stdio: ["pipe", "pipe", "pipe"],
      });
      child.stdin.on("error", () => {});
      child.stdin.end(input ?? "");
      let stdout = "";
      let stderr = "";
      child.stdout.setEncoding("utf8").on("data", (chunk: string) => {
        stdout += chunk;
      });
      child.stderr.setEncoding("utf8").on("data", (chunk: string) => {
        stderr += chunk;
      });
      child.on("error", () => reject(new OnetaskgraphExecutionError(null)));
      child.on("close", (code) => {
        // An exit this command does not answer with is an execution failure whatever
        // stdout holds: under `--json` a failed command also writes its failure document
        // there, which is not the response this command's schema describes.
        if (
          stdout.length === 0 ||
          (code !== 0 && (code !== 4 || !partialResponseCommands.has(command)))
        ) {
          reject(new OnetaskgraphExecutionError(code, stderr));
          return;
        }
        let value: unknown;
        try {
          value = JSON.parse(stdout);
        } catch {
          reject(new OnetaskgraphValidationError(command, "invalid JSON"));
          return;
        }
        if (command === "schema") {
          if (
            typeof value !== "object" ||
            value === null ||
            !("version" in value) ||
            value.version !== SCHEMA_BUNDLE_VERSION ||
            !("roots" in value) ||
            typeof value.roots !== "object" ||
            value.roots === null ||
            Array.isArray(value.roots) ||
            Object.values(value.roots).some(
              (schema) =>
                typeof schema !== "object" ||
                schema === null ||
                !("$schema" in schema) ||
                typeof schema.$schema !== "string",
            ) ||
            !("commands" in value) ||
            !Array.isArray(value.commands) ||
            value.commands.length !== binaryCommands.length ||
            value.commands.some((entry, index) => entry !== binaryCommands[index])
          ) {
            reject(new OnetaskgraphValidationError(command, "invalid bundle"));
            return;
          }
        } else {
          const root = responseRoots[command];
          if (root === undefined) {
            reject(new OnetaskgraphValidationError(command, "command has no response schema"));
            return;
          }
          const validate = new Ajv2020({ strict: false }).compile(runtimeSchemas[root]);
          if (!validate(value)) {
            reject(new OnetaskgraphValidationError(command, validate.errors));
            return;
          }
        }
        // The command-specific runtime schema has established T before this boundary returns it.
        resolvePromise(value as T);
      });
    });
  }
}

export function assertCompleteCommandSurface(): void {
  const missing = binaryCommands.filter((command) => {
    const method = command.replace(/[ -]([a-z])/g, (_, letter: string) => letter.toUpperCase());
    // The emitted runtime contract supplies a dynamic string, so the assertion is the
    // boundary that permits reflective lookup while this check verifies the method exists.
    return typeof OnetaskgraphClient.prototype[method as keyof OnetaskgraphClient] !== "function";
  });
  if (missing.length > 0)
    throw new Error(`client has no method for binary command '${missing[0]}'`);
}
