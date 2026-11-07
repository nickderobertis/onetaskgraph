// DO NOT MERGE. Run rust-cache's own restore and save entry points as its setup-rust-toolchain
// step would, the cache service answered by a loopback peer that holds and accepts nothing: a
// miss, so `target` is left as built; then its post step's cleanup and archive, whose size it logs.
import { execFileSync, spawn } from "node:child_process";
import { mkdtempSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";

const actions = join(process.env.RUNNER_WORKSPACE, "..", "_actions", "Swatinem", "rust-cache");
const dist = join(actions, readdirSync(actions)[0], "dist");
const server = createServer((_request, response) => {
  response.writeHead(204);
  response.end();
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const scratch = mkdtempSync(join(tmpdir(), "demo-archive-"));
const url = `http://127.0.0.1:${server.address().port}/`;
const env = {
  ...process.env,
  ACTIONS_RESULTS_URL: url,
  ACTIONS_RUNTIME_TOKEN: "demo",
  ACTIONS_CACHE_SERVICE_V2: "true",
  "INPUT_SHARED-KEY": "test-linux",
  "INPUT_SAVE-IF": "true",
  "INPUT_ADD-RUST-ENVIRONMENT-HASH-KEY": "true",
  "INPUT_ADD-JOB-ID-KEY": "true",
  "INPUT_CACHE-TARGETS": "true",
  "INPUT_CACHE-ON-FAILURE": "false",
  "INPUT_CACHE-ALL-CRATES": "false",
  "INPUT_CACHE-WORKSPACE-CRATES": "false",
  "INPUT_CACHE-BIN": "true",
  "INPUT_PREFIX-KEY": "v0-rust",
  "INPUT_CACHE-PROVIDER": "github",
  "INPUT_CMD-FORMAT": "{0}",
  INPUT_WORKSPACES: ".",
  GITHUB_STATE: join(scratch, "state"),
  GITHUB_OUTPUT: join(scratch, "output"),
};
writeFileSync(env.GITHUB_STATE, "");
writeFileSync(env.GITHUB_OUTPUT, "");
function run(entry, extra) {
  return new Promise((resolve) => {
    const child = spawn(process.execPath, [join(dist, entry, "index.js")], {
      env: { ...env, ...extra },
      stdio: ["ignore", "pipe", "inherit"],
    });
    child.stdout.on("data", (chunk) => process.stdout.write(chunk));
    child.on("close", resolve);
  });
}
await run("restore");
const state = readFileSync(env.GITHUB_STATE, "utf8");
const config = state.match(/RUST_CACHE_CONFIG<<(\S+)\n([\s\S]*?)\n\1/)?.[2];
if (!config) throw Error(`no rust-cache state: ${state.slice(0, 400)}`);
console.log(`key ${JSON.parse(config).cacheKey}`);
await run("save", { STATE_RUST_CACHE_CONFIG: config });
server.close();
execFileSync("true");
