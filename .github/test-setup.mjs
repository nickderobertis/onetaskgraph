import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fixtureEntryEnvironment } from "./fixture-environment.mjs";

// Run before test helpers load: even a child started without an env option must
// never inherit the outer Nx job's recording location or remote-cache authority.
for (const key of Object.keys(fixtureEntryEnvironment)) delete process.env[key];
const ownTemp = mkdtempSync(
  join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir(), "test-context-"),
);
process.env.RUNNER_TEMP = ownTemp;
process.once("exit", () => rmSync(ownTemp, { recursive: true, force: true }));
