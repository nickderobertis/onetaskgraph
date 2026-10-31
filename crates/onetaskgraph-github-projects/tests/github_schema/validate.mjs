// GitHub's static validation of a GraphQL request, by graphql-js — the GraphQL Foundation's
// reference implementation — against GitHub's complete published schema. `mod.rs` beside this
// file starts it once per test process and speaks to it one JSON line per request; see there
// for why it is graphql-js and what it answers.
//
// Usage: bun validate.mjs <schema.graphql>
// Writes `{"ready":true}` once the schema is built, then answers each request line
// `{"query", "operationName", "variables", "coerce"}` with `{"errors":[…]}`, empty when the
// request is valid. A failure that is not a refusal answers `{"internal": "<what>"}`.

import { readFileSync } from "node:fs";
import { createInterface } from "node:readline";
import {
  buildASTSchema,
  GraphQLError,
  getOperationAST,
  getVariableValues,
  parse,
  validate,
  validateSchema,
} from "graphql";

// The one type-system rule GitHub's published schema breaks: graphql-js 17 refuses an
// implementation field deprecated where its interface field is not. It governs how a schema is
// written, not what a request may ask, so a schema breaking it alone is built as valid; a schema
// breaking any other rule is refused, because then the copy is not the schema GitHub serves.
const TOLERATED_SCHEMA_ERROR =
  /^Interface field \S+ is not deprecated, so implementation field \S+ must not be deprecated\.$/;

// The most tokens a request may hold before it is refused unparsed: far beyond this source's
// largest batched document.
const MAX_TOKENS = 200_000;

const write = (answer) => process.stdout.write(`${JSON.stringify(answer)}\n`);

const schemaPath = process.argv[2];
let schema;
try {
  const sdl = parse(readFileSync(schemaPath, "utf8"));
  // Checked as built without `assumeValid`, which would record no errors for `validateSchema`
  // to report; the schema requests are validated against is then built with it, because
  // `validate` refuses to run against a schema with any error, the tolerated one included.
  const untolerated = validateSchema(buildASTSchema(sdl)).filter(
    (error) => !TOLERATED_SCHEMA_ERROR.test(error.message),
  );
  if (untolerated.length > 0) {
    write({
      internal: `the schema at ${schemaPath} is invalid: ${untolerated.map((error) => error.message).join("; ")}`,
    });
    process.exit(1);
  }
  schema = buildASTSchema(sdl, { assumeValid: true });
} catch (error) {
  write({ internal: `the schema at ${schemaPath} does not build: ${error}` });
  process.exit(1);
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

write({ ready: true });
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
