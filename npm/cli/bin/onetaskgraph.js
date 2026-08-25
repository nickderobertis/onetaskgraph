#!/usr/bin/env node
const { spawnSync } = require("node:child_process");
const { readFileSync, realpathSync } = require("node:fs");
const { join } = require("node:path");
const key = `${process.platform}-${process.arch}`;
const packages = { "linux-x64": "linux-x64", "linux-arm64": "linux-arm64", "darwin-x64": "darwin-x64", "darwin-arm64": "darwin-arm64", "win32-x64": "win32-x64" };
if (!packages[key]) { console.error(`onetaskgraph: unsupported platform ${key}; install with cargo instead`); process.exit(64); }
let manifestPath;
try { manifestPath = require.resolve(`@onetaskgraph/cli-${packages[key]}/package.json`); } catch (_) {
  console.error(`onetaskgraph: platform package @onetaskgraph/cli-${packages[key]} is not installed; reinstall @onetaskgraph/cli`); process.exit(69);
}
const expectedPackage = `@onetaskgraph/cli-${packages[key]}`;
let command;
try {
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  if (manifest.name !== expectedPackage) throw new Error(`resolved carrier identifies itself as ${manifest.name || "unnamed"}`);
  const packageRoot = realpathSync(join(manifestPath, ".."));
  command = realpathSync(join(packageRoot, "bin", process.platform === "win32" ? "onetaskgraph.exe" : "onetaskgraph"));
  if (!command.startsWith(`${packageRoot}/`) && !command.startsWith(`${packageRoot}\\`)) throw new Error("carrier binary escapes its package");
} catch (error) {
  console.error(`onetaskgraph: invalid ${expectedPackage}: ${error.message}; reinstall the platform package`); process.exit(69);
}
const result = spawnSync(command, process.argv.slice(2), { stdio: "inherit" });
if (result.error) { console.error(`onetaskgraph: ${result.error.message}; reinstall the platform package`); process.exit(69); }
if (result.status === null) { console.error(`onetaskgraph: carrier terminated by ${result.signal || "a signal"}`); process.exit(70); }
process.exit(result.status);
