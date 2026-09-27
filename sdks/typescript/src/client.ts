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
  Regenerated,
  RenderedTemplate,
  SourceListings,
  StatusCategory,
  StatusOptionsReport,
  TaskContentSet,
  TaskDetail,
  TaskPrioritySet,
  TaskStatusSet,
  TaskUpdated,
  TemplateAnswers,
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
  // A loader document's path, naming the template in place of `file`.
  templateLoader?: string;
};
// Which template a create or a render uses: a file (with its search path), or a loader
// document's path — the JSON a caller states its own resolved template with.
export type TemplateSourceOptions = TemplateOptions & {
  template?: string;
  templateLoader?: string;
  answers?: Record<string, JsonValue>;
  vars?: Record<string, string>;
};
// What every create names about the item besides its title and project. `body` is written to
// the binary's standard input and `bodyFile` read by it; neither beside a template, and never
// a `body` beside `answers`, which would share that one standard input.
export type CreateOptions = TemplateSourceOptions & {
  body?: string;
  bodyFile?: string;
  labels?: string[];
  repositories?: string[];
  metadata?: Record<string, JsonValue>;
};
export type TaskCreateOptions = CreateOptions & {
  status?: StatusCategory;
  dependsOn?: string[];
  delivers?: string[];
};
export type DocumentCreateOptions = CreateOptions & { id?: string };
// A targeted update: every field named is written, and nothing else. `bodyFile` is read by the
// binary byte for byte and replaces the content; `statusName` is the status's own word, for a
// source that keeps one. A list given replaces that list, `[]` included — which is how a list is
// cleared — and one left out is not named.
export type TaskUpdateOptions = {
  title?: string;
  bodyFile?: string;
  status?: StatusCategory;
  statusName?: string;
  priority?: Priority;
  metadata?: Record<string, JsonValue>;
  removeMetadata?: string[];
  delivers?: string[];
  dependsOn?: string[];
};
// A regenerate: the template to use in place of the recorded one, answers laid over the stored
// base, `unset` names whose answer is dropped, and `dryRun` to write nothing.
export type RenderOptions = TemplateSourceOptions & { unset?: string[]; dryRun?: boolean };
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

// Which schema root each command's machine output is, validated on every call against what the
// binary really wrote. Exported so the table can be reconciled with the Python generator's own,
// which is checked against the binary on every generation: see tests/generator.test.ts.
export const commandResponseRoots: Readonly<Record<string, keyof typeof runtimeSchemas>> = {
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
  "task update": "TaskUpdated",
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
  // A created task as `task show` answers with it, and a created document as `document show`.
  "task create": "TaskDetail",
  "task render": "Regenerated",
  "task answers": "TemplateAnswers",
  "document create": "QueryResponseOfQualifiedDocument",
  "document render": "Regenerated",
  "document answers": "TemplateAnswers",
};

// Exit 4 is a whole answer with part of it missing: a read some sources could not answer, or a
// write — a copy, or `task status set` — that landed and could not keep a task it delivers in
// step, which its response names. A comment verb is one call to one source and neither reads
// several nor keeps anything in step, so exit 4 is not a code it can produce and not one this
// client accepts from it. A `metadata set` is the same: one write to one source, and metadata is
// not status, so it keeps no delivered task in step — and so are `priority set` and `content
// set`, for the same reason. `sources fields` sets up one board and answers for it whole, or
// fails. A template verb reads no source at all. Of the verbs that create and regenerate from
// one, `task create` alone keeps what it delivers in step, so it alone can exit 4.
const partialResponseCommands = new Set(
  Object.keys(commandResponseRoots).filter(
    (command) =>
      command !== "config show" &&
      command !== "sources list" &&
      command !== "sources fields" &&
      !command.startsWith("task comment ") &&
      !command.endsWith(" metadata set") &&
      !command.endsWith(" priority set") &&
      !command.endsWith(" content set") &&
      !command.startsWith("template ") &&
      !command.endsWith(" render") &&
      !command.endsWith(" answers") &&
      command !== "document create",
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
  // Absent options are the default `{}`. Anything but a plain object is refused: a primitive
  // or `null` would fail on the property read below naming neither the argument nor what to
  // pass, and an array or a class instance would be read as no options at all.
  const prototype =
    options !== null && typeof options === "object" ? Object.getPrototypeOf(options) : 0;
  if (prototype !== Object.prototype && prototype !== null) {
    throw new TypeError(
      "templateVariables/templateRender: options is not a plain object; next: pass an " +
        "options object, or omit it",
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

// Refuse anything but a list of strings for one repeated flag, by index so a sparse array's
// hole is refused rather than skipped, and with no key but its entries, which is all that is
// sent.
function stringList(method: string, name: string, values: unknown): string[] {
  const list = values === undefined ? [] : values;
  if (!Array.isArray(list)) {
    throw new TypeError(`${method}: ${name} is not an array; next: pass a list of strings`);
  }
  refuseUncarriedKey(list, name, "entry", method);
  const checked: string[] = [];
  for (let index = 0; index < list.length; index += 1) {
    const value: unknown = list[index];
    if (typeof value !== "string") {
      throw new TypeError(
        `${method}: ${name}[${index}] is not a string; next: pass each entry as a string`,
      );
    }
    checked.push(value);
  }
  return checked;
}

// Whether `value` is a mapping written as a plain object — not an array, a class instance or a
// primitive, any of which would have its entries read as something they are not.
function isPlainObject(value: unknown): value is Record<string, unknown> {
  if (value === null || typeof value !== "object") return false;
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

// `vars` as `--var NAME=VALUE` flags, each value refused unless it is the text a command line
// would take.
function varFlags(method: string, given: unknown): string[] {
  // Absent means none; an explicit `null` is not a mapping and is refused below.
  const vars = given === undefined ? {} : given;
  if (!isPlainObject(vars)) {
    throw new TypeError(
      `${method}: vars is not a plain object; next: pass a mapping of variable name to text`,
    );
  }
  refuseUncarriedKey(vars, "vars", "var", method);
  const flags: string[] = [];
  for (const [name, value] of Object.entries(vars)) {
    // Checked rather than interpolated whatever it is: a number or an object would reach the
    // binary as its string form, which is not the value the caller passed.
    if (typeof value !== "string") {
      throw new TypeError(
        `${method}: vars.${name} is not a string; next: pass the text the command line ` +
          "would take, or give a typed value in answers",
      );
    }
    flags.push("--var", `${name}=${value}`);
  }
  return flags;
}

// A path option, refused unless it is one, for `templateFile`'s reasons — and never `-`, which
// would hand the binary's one standard input to a file the client does not write there.
function pathOption(method: string, name: string, value: unknown): string {
  if (typeof value !== "string" || value.length === 0 || value.startsWith("-")) {
    throw new TypeError(
      `${method}: ${name} is not a path; next: pass it as a non-empty string, spelling one ` +
        "that starts with `-` as `./-…`",
    );
  }
  return value;
}

// The flags naming a create's or a render's template and its answers, and what goes to
// standard input: the answers as JSON, or a create's plain `body` — never both.
function templateSourceArguments(
  method: string,
  options: TemplateSourceOptions & { body?: string },
): { args: string[]; input: string | undefined } {
  // Every create and render reads its options through here first, so this is where one that is
  // not an options object is refused, rather than having its values silently not read.
  if (!isPlainObject(options)) {
    throw new TypeError(`${method}: options is not a plain object; next: pass an options object`);
  }
  const args: string[] = [];
  if (options.template !== undefined) {
    args.push("--template", pathOption(method, "template", options.template));
  }
  if (options.templateLoader !== undefined) {
    args.push("--template-loader", pathOption(method, "templateLoader", options.templateLoader));
  }
  if (options.body !== undefined && typeof options.body !== "string") {
    throw new TypeError(`${method}: body is not a string; next: pass the body as text`);
  }
  for (const directory of stringList(method, "searchPath", options.searchPath)) {
    args.push("--search-path", directory);
  }
  args.push(...varFlags(method, options.vars));
  if (options.answers !== undefined && options.body !== undefined) {
    throw new TypeError(
      `${method}: body and answers both go to standard input; next: pass a body without a ` +
        "template, or answers with one",
    );
  }
  if (options.answers !== undefined) {
    args.push("--answers", "-");
    return { args, input: jsonDocument(options.answers, method, "answers") };
  }
  return { args, input: options.body };
}

// Every flag a create shares, and what goes to standard input.
function createArguments(
  method: string,
  project: string,
  title: string,
  options: CreateOptions,
): { args: string[]; input: string | undefined } {
  const { args, input } = templateSourceArguments(method, options);
  args.push("--project", project, "--title", title);
  if (options.bodyFile !== undefined) {
    args.push("--body-file", pathOption(method, "bodyFile", options.bodyFile));
  }
  // The binary reads a body from standard input only when nothing else names one, so a body
  // beside a template, a loader document or a body file would be dropped in silence.
  if (
    options.body !== undefined &&
    (options.template !== undefined ||
      options.templateLoader !== undefined ||
      options.bodyFile !== undefined)
  ) {
    throw new TypeError(
      `${method}: body is read only when no template, templateLoader or bodyFile names the ` +
        "body; next: pass one of them, not both",
    );
  }
  for (const label of stringList(method, "labels", options.labels)) args.push("--label", label);
  for (const repository of stringList(method, "repositories", options.repositories)) {
    args.push("--repository", repository);
  }
  if (options.metadata !== undefined) {
    // Checked as a whole first, for the reasons an answers document is: a value JSON cannot
    // carry would otherwise be dropped or changed on its way to the binary.
    const checked: Record<string, JsonValue> = JSON.parse(
      jsonDocument(options.metadata, method, "metadata"),
    );
    for (const [key, value] of Object.entries(checked)) {
      args.push("--metadata", `${key}=${JSON.stringify(value)}`);
    }
  }
  return { args, input };
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
function refuseUncarriedKey(
  value: object,
  path: string,
  entry: string,
  method = "templateRender",
): void {
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
      `${method}: ${path} has the key ${String(uncarried)}, which is not sent; next: ` +
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
  return jsonDocument(answers, "templateRender", "answers");
}

// `value` as one JSON document, refused by `method` naming `name` when it is not a mapping of
// JSON values — for the reasons `answersDocument` states.
function jsonDocument(document: unknown, method: string, name: string): string {
  const entry = name === "answers" ? "answer" : "entry";
  const within = new Set<object>();
  const copy = (value: unknown, path: string): JsonValue => {
    if (value === null || typeof value === "string" || typeof value === "boolean") return value;
    if (typeof value === "number" && Number.isFinite(value)) return value;
    if (typeof value === "object") {
      const prototype = Object.getPrototypeOf(value);
      const plain = Array.isArray(value) || prototype === Object.prototype || prototype === null;
      if (plain && !within.has(value)) {
        within.add(value);
        refuseUncarriedKey(value, `${name}${path}`, entry, method);
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
      `${method}: ${name}${path} is not a JSON value; next: pass strings, finite ` +
        "numbers, booleans, null, arrays and plain objects, with no cycle",
    );
  };
  // An answers document is a mapping: an array would pass the element checks below and reach
  // the binary as a document it refuses for its shape rather than for what is in it.
  const prototype =
    document !== null && typeof document === "object" ? Object.getPrototypeOf(document) : undefined;
  if (prototype !== Object.prototype && prototype !== null) {
    throw new TypeError(
      `${method}: ${name} is not a plain object; next: pass a mapping of name to value`,
    );
  }
  return JSON.stringify(copy(document, ""));
}

// A `template` verb's file operand, left out when a loader document names the template instead.
function templateOperand(
  method: string,
  file: unknown,
  options: { templateLoader?: string },
): string[] {
  if (file === undefined && options.templateLoader !== undefined) return [];
  return [templateFile(method, file)];
}

function loaderFlags(method: string, options: { templateLoader?: string }): string[] {
  return options.templateLoader === undefined
    ? []
    : ["--template-loader", pathOption(method, "templateLoader", options.templateLoader)];
}

// Answered as exactly the three arguments `run` takes, so both regenerate methods pass them
// through unchanged and cannot assemble the same invocation two ways.
function renderInvocation(
  command: string,
  method: string,
  id: string,
  options: RenderOptions,
): [string, string[], string | undefined] {
  // A render reads no body: the binary renders one, so a `body` that reached here untyped would
  // be written to standard input and never read.
  if (isPlainObject(options) && (options as { body?: unknown }).body !== undefined) {
    throw new TypeError(
      `${method}: body is not an option of a render; next: pass answers or vars, which the ` +
        "template renders the body from",
    );
  }
  const { args, input } = templateSourceArguments(method, options);
  for (const name of stringList(method, "unset", options.unset)) args.push("--unset", name);
  if (options.dryRun !== undefined && typeof options.dryRun !== "boolean") {
    throw new TypeError(`${method}: dryRun is not a boolean; next: pass true or false`);
  }
  if (options.dryRun === true) args.push("--dry-run");
  return [command, [id, ...args], input];
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
  // One targeted update of one task: at least one field has to be named.
  taskUpdate(id: string, options: TaskUpdateOptions): Promise<TaskUpdated> {
    const method = "taskUpdate";
    const args = [id];
    if (options.title !== undefined) args.push("--title", options.title);
    if (options.bodyFile !== undefined) {
      args.push("--body-file", pathOption(method, "bodyFile", options.bodyFile));
    }
    if (options.status !== undefined) args.push("--status", options.status);
    if (options.statusName !== undefined) args.push("--status-name", options.statusName);
    if (options.priority !== undefined) args.push("--priority", options.priority);
    if (options.metadata !== undefined) {
      // Checked as a whole first, as a create's metadata is.
      const checked: Record<string, JsonValue> = JSON.parse(
        jsonDocument(options.metadata, method, "metadata"),
      );
      for (const [key, value] of Object.entries(checked)) {
        args.push("--metadata", `${key}=${JSON.stringify(value)}`);
      }
    }
    for (const key of stringList(method, "removeMetadata", options.removeMetadata)) {
      args.push("--remove-metadata", key);
    }
    for (const [name, flag, values] of [
      ["delivers", "--delivers", options.delivers],
      ["dependsOn", "--depends-on", options.dependsOn],
    ] as const) {
      if (values === undefined) continue;
      const ids = stringList(method, name, values);
      if (ids.length === 0) args.push(flag === "--delivers" ? "--no-delivers" : "--no-depends-on");
      for (const id of ids) args.push(flag, id);
    }
    return this.run("task update", args);
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

  // `file` may be left out when `templateLoader` names the template in its place.
  async templateVariables(
    file?: string,
    options: TemplateOptions & { templateLoader?: string } = {},
  ): Promise<TemplateVariables> {
    return this.run("template variables", [
      ...templateOperand("templateVariables", file, options),
      ...searchPathFlags(options),
      ...loaderFlags("templateVariables", options),
    ]);
  }
  // The answers go over standard input as JSON, which is YAML, so no file is written for them.
  async templateRender(
    file?: string,
    options: TemplateRenderOptions = {},
  ): Promise<RenderedTemplate> {
    const args = [
      ...templateOperand("templateRender", file, options),
      ...searchPathFlags(options),
      ...loaderFlags("templateRender", options),
      ...varFlags("templateRender", options.vars),
    ];
    if (options.answers === undefined) return this.run("template render", args);
    const document = answersDocument(options.answers);
    args.push("--answers", "-");
    return this.run("template render", args, document);
  }

  // Create a task from a template, a loader document, `bodyFile` or `body`; it answers as
  // `taskShow` does, and records `onetaskgraph.template` provenance when rendered.
  async taskCreate(
    source: string,
    project: string,
    title: string,
    options: TaskCreateOptions = {},
  ): Promise<TaskDetail> {
    const { args, input } = createArguments("taskCreate", project, title, options);
    if (options.status !== undefined) args.push("--status", options.status);
    for (const id of stringList("taskCreate", "dependsOn", options.dependsOn)) {
      args.push("--depends-on", id);
    }
    for (const id of stringList("taskCreate", "delivers", options.delivers)) {
      args.push("--delivers", id);
    }
    return this.run("task create", [source, ...args], input);
  }
  // Regenerate one task in place from its template, over its stored answers.
  async taskRender(id: string, options: RenderOptions = {}): Promise<Regenerated> {
    return this.run(...renderInvocation("task render", "taskRender", id, options));
  }
  taskAnswers(id: string): Promise<TemplateAnswers> {
    return this.run("task answers", [id]);
  }
  // Create — or, with `id` naming one the source holds, replace — a project document.
  async documentCreate(
    source: string,
    project: string,
    title: string,
    options: DocumentCreateOptions = {},
  ): Promise<QueryResponseOfQualifiedDocument> {
    const { args, input } = createArguments("documentCreate", project, title, options);
    if (options.id !== undefined) args.push("--id", options.id);
    return this.run("document create", [source, ...args], input);
  }
  async documentRender(id: string, options: RenderOptions = {}): Promise<Regenerated> {
    return this.run(...renderInvocation("document render", "documentRender", id, options));
  }
  documentAnswers(id: string): Promise<TemplateAnswers> {
    return this.run("document answers", [id]);
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
          const root = commandResponseRoots[command];
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
