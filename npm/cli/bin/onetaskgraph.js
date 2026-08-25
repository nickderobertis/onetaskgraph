#!/usr/bin/env node
const { spawnSync } = require("node:child_process");
const { readFileSync, realpathSync } = require("node:fs");
const { join } = require("node:path");
const key = `${process.platform}-${process.arch}`;
// llmlint: ignore[changed_behavior_has_e2e] Exercising every native mapping requires Linux ARM64 and Darwin x64 runners, other platform actions unavailable to this pre-publication dispatch.
const packages = { "linux-x64": "linux-x64", "linux-arm64": "linux-arm64", "darwin-x64": "darwin-x64", "darwin-arm64": "darwin-arm64", "win32-x64": "win32-x64" };
if (!packages[key]) { console.error(`onetaskgraph: unsupported platform ${key}; install with cargo instead`); process.exit(64); }
const expectedPackage = `@onetaskgraph/cli-${packages[key]}`;
let manifestPath;
try { manifestPath = require.resolve(`${expectedPackage}/package.json`); } catch (error) {
  if (error.code === "MODULE_NOT_FOUND") console.error(`onetaskgraph: platform package ${expectedPackage} is not installed; reinstall @onetaskgraph/cli`);
  else console.error(`onetaskgraph: invalid ${expectedPackage}: ${error.message}; reinstall the platform package`);
  process.exit(69);
}
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
