// Every test file that starts the binary imports this first, for what importing it does.
//
// A client hands the binary this process's environment, and the binary reads every variable
// under this prefix as configuration: a shell that exports a source adds it to every command a
// test sends, and an assertion on the first row reads that source instead of the one the test
// configured. So every variable under the prefix is removed, as the binary's own journeys remove
// them, and nothing wider — clearing the whole environment would take `PATH` with it.
// `client.test.ts` holds this prefix to the one the binary really reads.

export const CONFIGURATION_PREFIX = "ONETASKGRAPH_";

for (const name of Object.keys(process.env)) {
  if (name.startsWith(CONFIGURATION_PREFIX)) delete process.env[name];
}
