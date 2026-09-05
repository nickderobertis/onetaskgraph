/**
 * The TypeScript SDK for onetaskgraph.
 *
 * Its declarations and runtime validators are generated from the schema bundle
 * `onetaskgraph schema` emits. The client drives that binary for every call, keeping one
 * implementation of the query semantics across the CLI and both SDKs.
 */

/** The version this package publishes. `package.json` must agree; see the tests. */
export const VERSION = "0.2.23";
export * from "./client.ts";
export type * from "./generated/models.ts";
