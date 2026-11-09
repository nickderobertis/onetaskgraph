// The workspace files a coverage unit reads outside its own crate, hashed for its Nx key.
//
//   node scripts/coverage-read-digest.mjs --unit NAME          one digest line
//   node scripts/coverage-read-digest.mjs --unit NAME --list   the files, one per line
//   node scripts/coverage-read-digest.mjs --rust               every tracked Rust source, as one
//                                                              digest line
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
const rustOnly = args[0] === "--rust" && args.length === 1;
const list = args[2] === "--list";
const unit = args[0] === "--unit" ? args[1] : undefined;
if (!rustOnly && (!unit || unit.startsWith("--") || args.length > (list ? 3 : 2)))
  refuse(
    "usage: node scripts/coverage-read-digest.mjs --unit NAME [--list]\nnext: pass the unit's name as Nx's runtime input does",
    2,
  );

let sets;
try {
  sets = JSON.parse(readFileSync("scripts/coverage-read-sets.json", "utf8"));
} catch (error) {
  refuse(
    `scripts/coverage-read-sets.json cannot be read: ${error.message}\nnext: restore it from git`,
    1,
  );
}
if (sets === null || typeof sets !== "object" || Array.isArray(sets))
  refuse("scripts/coverage-read-sets.json is not an object\nnext: restore it from git", 1);
const globs = rustOnly ? ["crates/**/*.rs"] : sets.units?.[unit];
const isGlobList = (value) =>
  Array.isArray(value) && value.every((entry) => typeof entry === "string" && entry !== "");
if (!isGlobList(globs) || !isGlobList(sets.common))
  refuse(
    `scripts/coverage-read-sets.json lists no pathspec globs for unit ${unit}\nnext: add the unit under "units" with the workspace files it reads`,
    1,
  );

const pathspecs = [...(rustOnly ? [] : sets.common), ...globs].map((glob) =>
  glob.startsWith("!") ? `:(exclude,glob)${glob.slice(1)}` : `:(glob)${glob}`,
);
let files;
try {
  files = execFileSync("git", ["ls-files", "-z", "--", ...pathspecs], {
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  })
    .split("\0")
    .filter(Boolean)
    .sort();
} catch (error) {
  refuse(
    `git could not list the tracked files: ${error.message}\nnext: run this from a checkout`,
    1,
  );
}

// llmlint: ignore[tool_output_is_signal] Standard output is this script's machine-read interface: Nx hashes this one line.
if (list) console.log(files.join("\n"));
else {
  const hash = createHash("sha256");
  try {
    for (const file of files)
      hash.update(file).update("\0").update(readFileSync(file)).update("\0");
  } catch (error) {
    refuse(`a read-set file cannot be read: ${error.message}\nnext: restore it from git`, 1);
  }
  console.log(hash.digest("hex"));
}
