// GitHub's static validation of a GraphQL request, by graphql-js — the GraphQL Foundation's
// reference implementation — against GitHub's complete published schema. `mod.rs` beside this
// file runs it once per machine and clone as a loopback server every test process shares, and
// speaks to it one JSON line per request; see there for why it is graphql-js and what it answers.
//
// Usage: bun validate.mjs <schema.graphql> [--serve <directory>]
// Writes `{"ready":true,"schema_sha256":"<hex>"}` once the schema is built, then answers each
// request line `{"query", "operationName", "variables", "coerce"}` with `{"errors":[…]}`, empty
// when the request is valid. A failure that is not a refusal answers `{"internal": "<what>"}`.
//
// Without `--serve` the requests are its standard input's lines. With it, the ready line also
// carries the `port` it listens on at 127.0.0.1, which it records in `<directory>/endpoint.json`
// for other processes to find; each connection opens with `{"hello": PROTOCOL}`, answered with
// the ready line — or `{"stale": "<why>"}` when this file, the schema or the tarball has changed
// since it started or the protocol differs, after which it stops so a current one can start —
// and its later lines are requests. It stops once no connection has been open for `IDLE_MS`.
//
// graphql-js is imported from the pinned tarball beside this file (see `mod.rs` for why).

import { createHash } from "node:crypto";
import {
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { createServer } from "node:net";
import { dirname, join, normalize, sep } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath, pathToFileURL } from "node:url";
import { gunzipSync } from "node:zlib";

// `https://registry.npmjs.org/graphql/-/graphql-17.0.2.tgz`, and the `dist.integrity` npm
// publishes for it (`npm view graphql@17.0.2 dist.integrity`).
const GRAPHQL_TARBALL = "graphql-17.0.2.tgz";
const GRAPHQL_INTEGRITY =
  "sha512-FRWbddMxfkjiB7z+aQDWIR+E34xo9I8c9mtK2RPv8PmMzKRvrdsreHL/Ui/TmwHJfhHChEtsFPyMHKI+xuarQQ==";

const write = (answer) => process.stdout.write(`${JSON.stringify(answer)}\n`);
const fail = (internal) => {
  write({ internal });
  process.exit(1);
};

// Each regular file of an npm tarball, by its path under `package/`. An npm tarball is a gzipped
// ustar archive of regular files alone; any other entry, or a path leaving the package, is
// refused rather than guessed at, which the pinned integrity makes unreachable.
function* tarFiles(archive) {
  const text = (offset, length) => {
    const field = archive.subarray(offset, offset + length);
    const end = field.indexOf(0);
    return field.subarray(0, end === -1 ? length : end).toString("utf8");
  };
  let offset = 0;
  while (offset + 512 <= archive.length && archive[offset] !== 0) {
    const name = text(offset, 100);
    const prefix = text(offset + 345, 155);
    const path = prefix ? `${prefix}/${name}` : name;
    const size = Number.parseInt(text(offset + 124, 12).trim() || "0", 8);
    const type = String.fromCharCode(archive[offset + 156] || 48);
    if (type !== "0" || !Number.isSafeInteger(size)) {
      throw new Error(`${path} is not a regular file (type ${type})`);
    }
    const relative = normalize(path.replace(/^package\//, ""));
    if (!path.startsWith("package/") || relative.startsWith("..") || relative.startsWith(sep)) {
      throw new Error(`${path} lies outside the package`);
    }
    yield [relative, archive.subarray(offset + 512, offset + 512 + size)];
    offset += 512 + Math.ceil(size / 512) * 512;
  }
}

// The directory graphql-js is imported from, unpacked from the pinned tarball. The tarball is
// held to its integrity on every start, and a copy already unpacked is imported only when it
// holds exactly the tarball's files, byte for byte — a marker alone vouches for nothing, since a
// changed file keeps it. Any other copy is replaced: unpacked beside its final place and renamed
// into it whole, so a process never imports a half-written copy, and concurrent first runs
// leave one.
//
// It is unpacked into `GITHUB_SCHEMA_MODULES` when that is set — `mod.rs` sets it to a directory
// under the workspace's `target`, so a test run writes nothing inside the crate it tests, whose
// directory a release hashes between building and publishing a crate — and into `node_modules`
// beside this file otherwise.
const here = dirname(fileURLToPath(import.meta.url));
const modules = process.env.GITHUB_SCHEMA_MODULES || join(here, "node_modules");

function graphqlPackage() {
  const target = join(modules, "graphql");
  const marker = join(target, ".onetaskgraph-integrity");
  const tarball = readFileSync(join(here, GRAPHQL_TARBALL));
  const integrity = `sha512-${createHash("sha512").update(tarball).digest("base64")}`;
  if (integrity !== GRAPHQL_INTEGRITY) {
    throw new Error(
      `${GRAPHQL_TARBALL} has integrity ${integrity}, not the ${GRAPHQL_INTEGRITY} npm publishes`,
    );
  }
  const files = new Map(tarFiles(gunzipSync(tarball)));
  const unpacked = () => {
    if (!existsSync(marker) || readFileSync(marker, "utf8") !== GRAPHQL_INTEGRITY) return false;
    const present = readdirSync(target, { recursive: true })
      .map(String)
      .filter((relative) => statSync(join(target, relative)).isFile());
    return (
      present.length === files.size + 1 &&
      present.every(
        (relative) =>
          relative === ".onetaskgraph-integrity" ||
          (files.has(relative) && readFileSync(join(target, relative)).equals(files.get(relative))),
      )
    );
  };
  if (unpacked()) return target;
  const staging = join(modules, `.graphql-${process.pid}-${Date.now()}`);
  rmSync(staging, { recursive: true, force: true });
  for (const [relative, contents] of files) {
    const file = join(staging, relative);
    mkdirSync(dirname(file), { recursive: true });
    writeFileSync(file, contents);
  }
  writeFileSync(join(staging, ".onetaskgraph-integrity"), GRAPHQL_INTEGRITY);
  // A copy that is not the tarball's is replaced; one another process has just renamed into
  // place is kept, and this process's own is discarded.
  if (existsSync(target) && !unpacked()) rmSync(target, { recursive: true, force: true });
  try {
    renameSync(staging, target);
  } catch (error) {
    rmSync(staging, { recursive: true, force: true });
    if (!unpacked()) throw error;
  }
  return target;
}

let graphql;
try {
  graphql = await import(pathToFileURL(join(graphqlPackage(), "index.mjs")).href);
} catch (error) {
  fail(`graphql-js could not be loaded from ${GRAPHQL_TARBALL}: ${error?.stack ?? error}`);
}
const {
  buildASTSchema,
  GraphQLError,
  getOperationAST,
  getVariableValues,
  parse,
  validate,
  validateSchema,
} = graphql;

// The one type-system rule GitHub's published schema breaks: graphql-js 17 refuses an
// implementation field deprecated where its interface field is not. It governs how a schema is
// written, not what a request may ask, so a schema breaking it alone is built as valid; a schema
// breaking any other rule is refused, because then the copy is not the schema GitHub serves.
const TOLERATED_SCHEMA_ERROR =
  /^Interface field \S+ is not deprecated, so implementation field \S+ must not be deprecated\.$/;

// The most tokens a request may hold before it is refused unparsed: far beyond this source's
// largest batched document.
const MAX_TOKENS = 200_000;

const schemaPath = process.argv[2];
let schema;
let schemaSha256;
try {
  const text = readFileSync(schemaPath);
  schemaSha256 = createHash("sha256").update(text).digest("hex");
  const sdl = parse(text.toString("utf8"));
  // Checked as built without `assumeValid`, which would record no errors for `validateSchema`
  // to report; the schema requests are validated against is then built with it, because
  // `validate` refuses to run against a schema with any error, the tolerated one included.
  const untolerated = validateSchema(buildASTSchema(sdl)).filter(
    (error) => !TOLERATED_SCHEMA_ERROR.test(error.message),
  );
  if (untolerated.length > 0) {
    fail(
      `the schema at ${schemaPath} is invalid: ${untolerated.map((error) => error.message).join("; ")}`,
    );
  }
  schema = buildASTSchema(sdl, { assumeValid: true });
} catch (error) {
  fail(`the schema at ${schemaPath} does not build: ${error}`);
}

const refusal = (message) => ({ errors: [{ message }] });

// Each GraphQL error as a response carries it (§7.1.2): its message and, where it stands in
// the document, its locations.
const answered = (errors) => ({
  errors: errors.map((error) => {
    const { message, locations } = error.toJSON();
    return locations ? { message, locations } : { message };
  }),
});

// The most documents `checked` keeps before it starts again from none: far more distinct
// documents than one test process sends, and a bound on what an adversarial test can make it hold.
const DOCUMENTS_KEPT = 4096;
const documents = new Map();

// `query` parsed and put to every validation rule — `{ document }` when it passes, `{ refused }`
// with the answer when it does not — kept by its text, because a test process sends the same few
// documents with many different variables and neither step depends on them.
function checked(query) {
  const kept = documents.get(query);
  if (kept) return kept;
  let result;
  try {
    const document = parse(query, { maxTokens: MAX_TOKENS });
    const invalid = validate(schema, document);
    result = invalid.length > 0 ? { refused: answered(invalid) } : { document };
  } catch (error) {
    if (!(error instanceof GraphQLError)) throw error;
    result = { refused: answered([error]) };
  }
  if (documents.size >= DOCUMENTS_KEPT) documents.clear();
  documents.set(query, result);
  return result;
}

// The request line's own shape, which `mod.rs` always sends whole: a string `query`, a string or
// null `operationName`, and an explicit boolean `coerce`. Anything else is a defect of the client,
// answered `internal` rather than read with defaults — a missing `coerce` would otherwise skip the
// coercion of the variables silently.
function envelope(request) {
  if (request === null || typeof request !== "object" || Array.isArray(request)) {
    throw new TypeError(`a request is a JSON object, not ${JSON.stringify(request)}`);
  }
  const { query, operationName, variables, coerce } = request;
  if (typeof query !== "string") {
    throw new TypeError(`a request's query is a string, not ${JSON.stringify(query)}`);
  }
  if (operationName !== null && operationName !== undefined && typeof operationName !== "string") {
    throw new TypeError(
      `a request's operationName is a string or null, not ${JSON.stringify(operationName)}`,
    );
  }
  if (typeof coerce !== "boolean") {
    throw new TypeError(`a request's coerce is true or false, not ${JSON.stringify(coerce)}`);
  }
  return { query, operationName, variables, coerce };
}

function answer(request) {
  const { query, operationName, variables, coerce } = envelope(request);
  const { document, refused } = checked(query);
  if (refused) return refused;
  if (!coerce) return { errors: [] };
  const operation = getOperationAST(document, operationName ?? undefined);
  if (!operation) {
    return refusal(
      operationName
        ? `Unknown operation named "${operationName}".`
        : "Must provide operation name if query contains multiple operations.",
    );
  }
  if (variables === null || typeof variables !== "object" || Array.isArray(variables)) {
    return refusal(
      "Variables must be provided as an Object where each property is a variable value.",
    );
  }
  const coerced = getVariableValues(schema, operation.variableDefinitions ?? [], variables);
  return coerced.errors ? answered(coerced.errors) : { errors: [] };
}

// One request line answered. A document or a value nested past what the parser, the validation
// or the coercion can recurse through exhausts the engine's stack, which JavaScript reports as a
// RangeError rather than a crash: the request is refused for it, as one too deep to run.
function answerLine(line) {
  try {
    return answer(JSON.parse(line));
  } catch (error) {
    return error instanceof RangeError
      ? refusal("The request nests too deeply to validate.")
      : { internal: String(error?.stack ?? error) };
  }
}

// The protocol `mod.rs` speaks; it sends this in its hello, so a client and a server built from
// different revisions of the two files cannot answer each other.
const PROTOCOL = 1;
// How long a server with no open connection waits before it stops: long enough to carry one test
// process's work to the next, short enough that nothing lingers once the tests are done.
const IDLE_MS = 10_000;

function serve(directory) {
  const endpoint = join(directory, "endpoint.json");
  // What this server was built from. A change to any of them makes it stale.
  const watched = [fileURLToPath(import.meta.url), schemaPath, join(here, GRAPHQL_TARBALL)];
  const stamp = () =>
    watched
      .map((file) => {
        try {
          const { size, mtimeMs } = statSync(file);
          return `${size}:${mtimeMs}`;
        } catch {
          return "absent";
        }
      })
      .join("|");
  const built = stamp();
  let open = 0;
  let idle;
  const stop = () => {
    server.close();
    try {
      if (JSON.parse(readFileSync(endpoint, "utf8")).pid === process.pid) rmSync(endpoint);
    } catch {
      // Already gone, or another server's: either way not this one's to remove.
    }
    process.exit(0);
  };
  const quiet = () => {
    clearTimeout(idle);
    if (open === 0) idle = setTimeout(stop, IDLE_MS);
  };
  const server = createServer((socket) => {
    open += 1;
    clearTimeout(idle);
    socket.setNoDelay(true);
    socket.on("error", () => {});
    socket.on("close", () => {
      open -= 1;
      quiet();
    });
    let greeted = false;
    const send = (value) => socket.write(`${JSON.stringify(value)}\n`);
    createInterface({ input: socket, crlfDelay: Infinity }).on("line", (line) => {
      if (greeted) {
        send(answerLine(line));
        return;
      }
      let hello;
      try {
        hello = JSON.parse(line).hello;
      } catch {
        hello = undefined;
      }
      const why =
        hello !== PROTOCOL
          ? `protocol ${PROTOCOL} was sent ${JSON.stringify(hello)}`
          : stamp() !== built
            ? `${watched.join(", ")} changed since it started`
            : undefined;
      if (why) {
        socket.end(`${JSON.stringify({ stale: why })}\n`, stop);
        return;
      }
      greeted = true;
      send({ ready: true, schema_sha256: schemaSha256 });
    });
  });
  server.on("error", (error) => fail(`the schema check could not listen: ${error}`));
  server.listen(0, "127.0.0.1", () => {
    const { port } = server.address();
    const staged = `${endpoint}.${process.pid}`;
    writeFileSync(staged, JSON.stringify({ port, pid: process.pid, protocol: PROTOCOL }));
    renameSync(staged, endpoint);
    write({ ready: true, schema_sha256: schemaSha256, port });
    quiet();
  });
}

if (process.argv[3] === "--serve") {
  serve(process.argv[4]);
} else {
  write({ ready: true, schema_sha256: schemaSha256 });
  for await (const line of createInterface({ input: process.stdin, crlfDelay: Infinity })) {
    write(answerLine(line));
  }
}
