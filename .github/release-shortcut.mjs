import { execFileSync } from "node:child_process";
import { appendFileSync, realpathSync } from "node:fs";
import { fileURLToPath } from "node:url";

// Rust coverage may be omitted only when main already proved the same source tree.
// Functional tests and SDK coverage still run. Unknown history or edits require coverage.
// It stays beside result replay because this is the tier replay cannot serve: raw Rust profiles
// and objects are uncached, so without it a release would re-instrument what main measured.
const RUN = "run";
const SKIP = "skip";
const decision = (rust_coverage, reason) => ({ schema_version: 1, rust_coverage, reason });

function git(args, cwd) {
  return execFileSync("git", args, { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] })
    .replaceAll("\r", "")
    .trim();
}

function tree(ref, cwd) {
  return git(["rev-parse", "--verify", "--end-of-options", `${ref}^{tree}`], cwd);
}

export function decide({ base, head, main, cwd = process.cwd() }) {
  let baseTree;
  let passed;
  try {
    baseTree = tree(base, cwd);
    tree(head, cwd);
    const tip = git(["rev-parse", "--verify", "--end-of-options", `${main}^{commit}`], cwd);
    passed = new Set(git(["log", "--first-parent", "--format=%T", tip, "--"], cwd).split("\n"));
  } catch {
    return decision(RUN, "Unreadable base, head or main requires Rust coverage");
  }
  if (!passed.has(baseTree)) return decision(RUN, `The base tree is not one ${main} passed`);
  try {
    const comparator = fileURLToPath(new URL("./semantic-release.py", import.meta.url));
    const releaseOnly = JSON.parse(
      execFileSync("python3", [comparator, base, head], {
        cwd,
        encoding: "utf8",
        stdio: ["ignore", "pipe", "pipe"],
      }),
    );
    if (releaseOnly === true)
      return decision(
        SKIP,
        `The tree is one ${main} passed plus only release-plz's version and changelog edits`,
      );
    if (releaseOnly === false)
      return decision(RUN, "The tree changes more than release-owned versions and changelogs");
    throw Error("Invalid semantic classifier result");
  } catch {
    return decision(RUN, "Unreadable release diff requires Rust coverage");
  }
}

// Both spellings resolved: `import.meta.url` names the file through no symlink, and argv need
// not (macOS's temporary directory is one), and a mismatch would decide nothing and exit 0.
if (
  process.argv[1] &&
  realpathSync(process.argv[1]) === realpathSync(fileURLToPath(import.meta.url))
) {
  try {
    const args = process.argv.slice(2);
    if (
      args.length !== 7 ||
      args[0] !== "decide" ||
      args[1] !== "--base" ||
      args[3] !== "--head" ||
      args[5] !== "--main"
    )
      throw Error(
        "usage: release-shortcut.mjs decide --base REF --head REF --main REF; next: pass the pull request's base, the commit under test and the default branch",
      );
    const result = decide({ base: args[2], head: args[4], main: args[6] });
    console.log(JSON.stringify(result));
    if (process.env.GITHUB_OUTPUT)
      appendFileSync(
        process.env.GITHUB_OUTPUT,
        `rust_coverage=${result.rust_coverage}\nreason=${result.reason}\n`,
      );
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
