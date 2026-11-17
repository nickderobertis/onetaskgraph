// Every test file that starts the binary imports this first, for what importing it does.
//
// A client hands the binary this process's environment, and the binary reads every variable
// under this prefix as configuration: a shell that exports a source adds it to every command a
// test sends, and an assertion on the first row reads that source instead of the one the test
// configured. So every variable under the prefix is removed, as the binary's own journeys remove
// them, and nothing wider — clearing the whole environment would take `PATH` with it.
// `client.test.ts` holds this prefix to the one the binary really reads.

import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";

export const CONFIGURATION_PREFIX = "ONETASKGRAPH_";

for (const name of Object.keys(process.env)) {
  if (name.startsWith(CONFIGURATION_PREFIX)) delete process.env[name];
}

// The binary also reads a user layer, `$XDG_CONFIG_HOME/onetaskgraph/config.yaml` and the
// `secrets.env` beside it, falling back to `$HOME/.config`. A host that keeps one — sources
// declared private and a `write_policy` turn the public boundary on — changes what a copy does,
// so a test passed in CI and failed on such a host. Every test starts from an empty
// configuration home instead; one that needs a user layer names its own `XDG_CONFIG_HOME`.
const configurationHome = mkdtempSync(resolve(tmpdir(), "onetaskgraph-sdk-config-home-"));
process.env.XDG_CONFIG_HOME = configurationHome;
process.on("exit", () => rmSync(configurationHome, { recursive: true, force: true }));
