// GitHub's static validation of a GraphQL request, by graphql-js — the GraphQL Foundation's
// reference implementation — against GitHub's complete published schema. `mod.rs` beside this
// file starts it once per test process and speaks to it one JSON line per request; see there
// for why it is graphql-js and what it answers.
//
// Usage: bun validate.mjs <schema.graphql>
// Writes `{"ready":true,"schema_sha256":"<hex>"}` once the schema is built, then answers each
// request line `{"query", "operationName", "variables", "coerce"}` with `{"errors":[…]}`, empty
// when the request is valid. A failure that is not a refusal answers `{"internal": "<what>"}`.
//
// graphql-js is imported from the pinned tarball beside this file (see `mod.rs` for why).

import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
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

// The directory graphql-js is imported from, unpacked from the pinned tarball the first time
// and found there afterwards. It is unpacked beside its final place and renamed into it whole,
// so a process never imports a half-written copy, and concurrent first runs leave one.
function graphqlPackage() {
  const here = dirname(fileURLToPath(import.meta.url));
  const target = join(here, "node_modules", "graphql");
  const marker = join(target, ".onetaskgraph-integrity");
  const unpacked = () => existsSync(marker) && readFileSync(marker, "utf8") === GRAPHQL_INTEGRITY;
  if (unpacked()) return target;
  const tarball = readFileSync(join(here, GRAPHQL_TARBALL));
  const integrity = `sha512-${createHash("sha512").update(tarball).digest("base64")}`;
  if (integrity !== GRAPHQL_INTEGRITY) {
    throw new Error(
      `${GRAPHQL_TARBALL} has integrity ${integrity}, not the ${GRAPHQL_INTEGRITY} npm publishes`,
    );
  }
  const staging = join(here, "node_modules", `.graphql-${process.pid}-${Date.now()}`);
  rmSync(staging, { recursive: true, force: true });
  for (const [relative, contents] of tarFiles(gunzipSync(tarball))) {
    const file = join(staging, relative);
    mkdirSync(dirname(file), { recursive: true });
    writeFileSync(file, contents);
  }
  writeFileSync(join(staging, ".onetaskgraph-integrity"), GRAPHQL_INTEGRITY);
  // A copy unpacked from another pin is replaced; one another process has just renamed into
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

function answer({ query, operationName, variables, coerce }) {
  let document;
  try {
    document = parse(query, { maxTokens: MAX_TOKENS });
  } catch (error) {
    if (error instanceof GraphQLError) return answered([error]);
    throw error;
  }
  const invalid = validate(schema, document);
  if (invalid.length > 0) return answered(invalid);
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

write({ ready: true, schema_sha256: schemaSha256 });
for await (const line of createInterface({ input: process.stdin, crlfDelay: Infinity })) {
  try {
    write(answer(JSON.parse(line)));
  } catch (error) {
    // A document or a value nested past what the parser, the validation or the coercion can
    // recurse through exhausts the engine's stack, which JavaScript reports as a RangeError
    // rather than a crash: the request is refused for it, as one too deep to run.
    write(
      error instanceof RangeError
        ? refusal("The request nests too deeply to validate.")
        : { internal: String(error?.stack ?? error) },
    );
  }
}
