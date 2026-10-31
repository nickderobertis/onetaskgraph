import { execFileSync } from "node:child_process";
import { appendFileSync, realpathSync } from "node:fs";
import { fileURLToPath } from "node:url";

// Whether a pull request's platform test jobs may succeed without running their tests.
//
// They may only when the tree under test is a tree `main` already passed plus nothing but
// release-plz's version and changelog edits — the release pull request, whose checks would
// otherwise prove again what main's already proved. The decision is the tree comparison
// alone: a branch name, a title or an author says nothing about what the tree holds, so none
// of them is read.
//
// "A tree main passed" is a tree of a commit on `main`'s first-parent history: branch
// protection admits a commit there only once its required checks concluded success.
//
// Any question this cannot answer is `run`, because running the tests is the direction a
// wrong answer is recoverable in.
const RUN = "run";
const SKIP = "skip";

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
    return { tests: RUN, reason: "Unreadable base, head or main requires the tests" };
  }
  if (!passed.has(baseTree))
    return { tests: RUN, reason: `The base tree is not one ${main} passed` };
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
      return {
        tests: SKIP,
        reason: `The tree is one ${main} passed plus only release-plz's version and changelog edits`,
      };
    if (releaseOnly === false)
      return {
        tests: RUN,
        reason: "The tree changes more than release-owned versions and changelogs",
      };
    throw Error("Invalid semantic classifier result");
  } catch {
    return { tests: RUN, reason: "Unreadable release diff requires the tests" };
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
      appendFileSync(process.env.GITHUB_OUTPUT, `tests=${result.tests}\nreason=${result.reason}\n`);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
