#!/usr/bin/env node
const { spawnSync } = require("node:child_process");
const { join } = require("node:path");
const key = `${process.platform}-${process.arch}`;
const packages = { "linux-x64": "linux-x64", "linux-arm64": "linux-arm64", "darwin-x64": "darwin-x64", "darwin-arm64": "darwin-arm64", "win32-x64": "win32-x64" };
if (!packages[key]) { console.error(`onetaskgraph: unsupported platform ${key}`); process.exit(1); }
let root;
try { root = require.resolve(`@onetaskgraph/cli-${packages[key]}/package.json`); } catch (_) {
  console.error(`onetaskgraph: platform package @onetaskgraph/cli-${packages[key]} is not installed`); process.exit(1);
}
const command = join(root, "..", "bin", process.platform === "win32" ? "onetaskgraph.exe" : "onetaskgraph");
const result = spawnSync(command, process.argv.slice(2), { stdio: "inherit" });
if (result.error) { console.error(`onetaskgraph: ${result.error.message}`); process.exit(1); }
process.exit(result.status === null ? 1 : result.status);
