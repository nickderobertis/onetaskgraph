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
  RenderedTemplate,
  SourceListings,
  StatusCategory,
  StatusOptionsReport,
  TaskContentSet,
  TaskDetail,
  TaskPrioritySet,
  TaskStatusSet,
  TemplateVariables,
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
// Where `extends`, `include` and `import` names resolve: these directories, in order, and never
// the working directory unless it is one of them.
export type TemplateOptions = { searchPath?: string[] };
// A template's answers: a mapping the client writes to the binary's standard input, and
// `vars` as `--var NAME=VALUE`, which outrank it — literal text for a `string` or `text`
// variable, YAML for any other.
export type TemplateRenderOptions = TemplateOptions & {
  answers?: Record<string, JsonValue>;
  vars?: Record<string, string>;
};
// A value JSON can carry, and so a value an answers document can hold.
export type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [key: string]: JsonValue };
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
  "template variables": "TemplateVariables",
  "template render": "RenderedTemplate",
};

// Exit 4 is a whole answer with part of it missing: a read some sources could not answer, or a
// write — a copy, or `task status set` — that landed and could not keep a task it delivers in
// step, which its response names. A comment verb is one call to one source and neither reads
// several nor keeps anything in step, so exit 4 is not a code it can produce and not one this
// client accepts from it. A `metadata set` is the same: one write to one source, and metadata is
// not status, so it keeps no delivered task in step — and so are `priority set` and `content
// set`, for the same reason. `sources fields` sets up one board and answers for it whole, or
// fails. A template verb reads no source at all.
const partialResponseCommands = new Set(
  Object.keys(responseRoots).filter(
    (command) =>
      command !== "config show" &&
      command !== "sources list" &&
      command !== "sources fields" &&
      !command.startsWith("task comment ") &&
      !command.endsWith(" metadata set") &&
      !command.endsWith(" priority set") &&
      !command.endsWith(" content set") &&
      !command.startsWith("template "),
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

function searchPathFlags(options: TemplateOptions): string[] {
  // Absent options are the default `{}`; anything else that is not an object would otherwise
  // fail on the property read below, naming neither the argument nor what to pass instead.
  if (options === null || typeof options !== "object") {
    throw new TypeError(
      "templateVariables/templateRender: options is not an object; next: pass an options " +
        "object, or omit it",
    );
  }
  // Absent means none; an explicit `null` is not a list and is refused below with the rest.
  const searchPath = options.searchPath === undefined ? [] : options.searchPath;
  if (!Array.isArray(searchPath)) {
    throw new TypeError(
      "templateVariables/templateRender: searchPath is not an array; next: pass a list of " +
        "directory path strings",
    );
  }
  const flags: string[] = [];
  // By index rather than by entry, so a hole in a sparse array is read as the `undefined` it
  // is and refused, rather than skipped and the search path quietly shortened.
  for (let index = 0; index < searchPath.length; index += 1) {
    const directory: unknown = searchPath[index];
    // Checked rather than passed on whatever it is: a process argument has to be text, and
    // anything else would reach the binary as its string form or fail the spawn.
    if (typeof directory !== "string") {
      throw new TypeError(
        `templateVariables/templateRender: searchPath[${index}] is not a string; next: pass ` +
          "each search directory as a path string",
      );
    }
    flags.push("--search-path", directory);
  }
  return flags;
}

// The template path, refused unless it is one: anything but a string would reach the binary as
// its string form, an empty one names no file, and one opening with `-` would be read as an
// option rather than as the file the caller named.
function templateFile(method: string, file: unknown): string {
  if (typeof file !== "string" || file.length === 0 || file.startsWith("-")) {
    throw new TypeError(
      `${method}: file is not a template path; next: pass the template's path as a non-empty ` +
        "string, spelling one that starts with `-` as `./-…`",
    );
  }
  return file;
}

// Refuse a key of `value` that what is sent would not carry: for an array anything but its
// indices below `length` and `length` itself, such as its own `toJSON` or a key spelled like
// an index past the largest an array has; for a mapping a symbol or a non-enumerable key,
// which `Object.entries` skips. Either would otherwise be dropped in silence, and the binary
// sent less than was handed over.
function refuseUncarriedKey(value: object, path: string, entry: string): void {
  const array = Array.isArray(value) ? value : undefined;
  const uncarried = Reflect.ownKeys(value).find((key) => {
    if (typeof key === "symbol") return true;
    if (array !== undefined) {
      return key !== "length" && !(/^(0|[1-9][0-9]*)$/.test(key) && Number(key) < array.length);
    }
    return !Object.prototype.propertyIsEnumerable.call(value, key);
  });
  if (uncarried !== undefined) {
    throw new TypeError(
      `templateRender: ${path} has the key ${String(uncarried)}, which is not sent; next: ` +
        `give every ${entry} an enumerable string key, and an array nothing but its entries`,
    );
  }
}

// The answers as the JSON document the binary reads on standard input, refused here when a
// value is not one JSON can carry: `JSON.stringify` would otherwise drop an `undefined` or a
// function without a word, write a non-finite number or an array's hole as `null`, and throw
// on a cycle or a bigint, so the binary would validate answers other than the ones the caller
// passed. What is serialised is a copy built from the values just checked, never the caller's
// own objects, so nothing the check did not read — a `toJSON` of an array's own, say — can
// change what is sent.
function answersDocument(answers: Record<string, JsonValue>): string {
  const within = new Set<object>();
  const copy = (value: unknown, path: string): JsonValue => {
    if (value === null || typeof value === "string" || typeof value === "boolean") return value;
    if (typeof value === "number" && Number.isFinite(value)) return value;
    if (typeof value === "object") {
      const prototype = Object.getPrototypeOf(value);
      const plain = Array.isArray(value) || prototype === Object.prototype || prototype === null;
      if (plain && !within.has(value)) {
        within.add(value);
        refuseUncarriedKey(value, `answers${path}`, "answer");
        let copied: JsonValue;
        if (Array.isArray(value)) {
          // By index rather than by entry, so a hole in a sparse array is read as the
          // `undefined` it is and refused, rather than skipped and later written as `null`.
          copied = Array.from({ length: value.length }, (_, index) =>
            copy(value[index], `${path}[${index}]`),
          );
        } else {
          copied = Object.fromEntries(
            Object.entries(value).map(([key, item]) => [key, copy(item, `${path}.${key}`)]),
          );
        }
        within.delete(value);
        return copied;
      }
    }
    throw new TypeError(
      `templateRender: answers${path} is not a JSON value; next: pass strings, finite ` +
        "numbers, booleans, null, arrays and plain objects, with no cycle",
    );
  };
  // An answers document is a mapping: an array would pass the element checks below and reach
  // the binary as a document it refuses for its shape rather than for what is in it.
  const prototype =
    answers !== null && typeof answers === "object" ? Object.getPrototypeOf(answers) : undefined;
  if (prototype !== Object.prototype && prototype !== null) {
    throw new TypeError(
      "templateRender: answers is not a plain object; next: pass a mapping of variable name to value",
    );
  }
  return JSON.stringify(copy(answers, ""));
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

  async templateVariables(file: string, options: TemplateOptions = {}): Promise<TemplateVariables> {
    return this.run("template variables", [
      templateFile("templateVariables", file),
      ...searchPathFlags(options),
    ]);
  }
  // The answers go over standard input as JSON, which is YAML, so no file is written for them.
  async templateRender(
    file: string,
    options: TemplateRenderOptions = {},
  ): Promise<RenderedTemplate> {
    const args = [templateFile("templateRender", file), ...searchPathFlags(options)];
    // Absent means none; an explicit `null` is not a mapping and is refused below.
    const vars = options.vars === undefined ? {} : options.vars;
    const prototype = vars !== null && typeof vars === "object" ? Object.getPrototypeOf(vars) : 0;
    if (prototype !== Object.prototype && prototype !== null) {
      throw new TypeError(
        "templateRender: vars is not a plain object; next: pass a mapping of variable name to text",
      );
    }
    refuseUncarriedKey(vars, "vars", "var");
    for (const [name, value] of Object.entries(vars)) {
      // Checked rather than interpolated whatever it is: a number or an object would reach the
      // binary as its string form, which is not the value the caller passed.
      if (typeof value !== "string") {
        throw new TypeError(
          `templateRender: vars.${name} is not a string; next: pass the text the command line ` +
            "would take, or give a typed value in answers",
        );
      }
      args.push("--var", `${name}=${value}`);
    }
    if (options.answers === undefined) return this.run("template render", args);
    const document = answersDocument(options.answers);
    args.push("--answers", "-");
    return this.run("template render", args, document);
  }

  private run<T>(command: string, args: string[], input?: string): Promise<T> {
    // Every call is non-interactive: a library caller has no terminal to be asked on, and a
    // command that would prompt refuses what it was not given instead of waiting.
    const commandArgs = [...command.split(" "), ...args, "--json", "--no-interactive"];
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
