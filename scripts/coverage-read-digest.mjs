// The workspace files a coverage unit reads outside its own crate, hashed for its Nx key.
//
//   node scripts/coverage-read-digest.mjs --unit NAME          one digest line
//   node scripts/coverage-read-digest.mjs --unit NAME --list   the files, one per line
//
// Reads: scripts/coverage-read-sets.json and the tracked files its pathspec globs name.
// Exit: 0 on success, 2 on any other arguments, 1 when the sets file or a tracked file cannot be
// read. A runtime input rather than Nx file inputs, because a `{workspaceRoot}` file input marks
// its project affected and selection rule 3 of AGENTS.md forbids a sibling plugin's e2e suite.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";

function refuse(message, status) {
  console.error(`coverage-read-digest: ${message}`);
  process.exit(status);
}

const args = process.argv.slice(2);
const list = args.includes("--list");
const unitAt = args.indexOf("--unit");
const unit = unitAt < 0 ? undefined : args[unitAt + 1];
const known = new Set(["--unit", "--list", unit]);
if (!unit || args.some((arg) => !known.has(arg)))
  refuse("usage: node scripts/coverage-read-digest.mjs --unit NAME [--list]", 2);

let sets;
try {
  sets = JSON.parse(readFileSync("scripts/coverage-read-sets.json", "utf8"));
} catch (error) {
  refuse(`scripts/coverage-read-sets.json cannot be read: ${error.message}`, 1);
}
const globs = sets?.units?.[unit];
if (!Array.isArray(globs) || !Array.isArray(sets.common))
  refuse(`scripts/coverage-read-sets.json names no unit ${unit}`, 1);

const pathspecs = [...sets.common, ...globs].map((glob) =>
  glob.startsWith("!") ? `:(exclude,glob)${glob.slice(1)}` : `:(glob)${glob}`,
);
const files = execFileSync("git", ["ls-files", "-z", "--", ...pathspecs], {
  encoding: "utf8",
  maxBuffer: 64 * 1024 * 1024,
})
  .split("\0")
  .filter(Boolean)
  .sort();

// llmlint: ignore[tool_output_is_signal] Standard output is this script's machine-read interface: Nx hashes this one line.
if (list) console.log(files.join("\n"));
else {
  const hash = createHash("sha256");
  for (const file of files) hash.update(file).update("\0").update(readFileSync(file)).update("\0");
  console.log(hash.digest("hex"));
}
