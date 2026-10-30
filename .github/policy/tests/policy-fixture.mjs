// The git history the macOS policy tests classify, built once per test file that imports it:
// node --test runs each file in a process of its own, side by side, and each gets its own
// scratch history, so the files share no state.
import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { read, root, validateEventInventory } from "../../workflow-graph.mjs";

validateEventInventory(JSON.parse(read(".github/event-classes.json")));

export const policy =
  process.env.MACOS_POLICY_TEST_PROGRAM ||
  fileURLToPath(new URL("../../macos-policy.mjs", import.meta.url));
export const scratchRoot = process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir();
export const history = mkdtempSync(resolve(scratchRoot, "macos-policy-"));
export function git(...args) {
  return execFileSync("git", args, { cwd: history, encoding: "utf8" }).trim();
}
export function put(path, text) {
  const dest = resolve(history, path);
  mkdirSync(resolve(dest, ".."), { recursive: true });
  writeFileSync(dest, text);
}
git("init", "-q");
git("config", "user.email", "policy@example.invalid");
git("config", "user.name", "Workflow contract");
put(".github/macos-result.mjs", read(".github/macos-result.mjs"));
put(".gitignore", "event.json\ngithub-output\nneeds.json\n.github/node_modules/\n");
for (const file of ["macos-policy.mjs", "semantic-release.py", "semantic-json.mjs"])
  put(`.github/${file}`, read(`.github/${file}`));
put("scripts/product_versions.py", read("scripts/product_versions.py"));
symlinkSync(
  fileURLToPath(new URL(".github/node_modules", root)),
  resolve(history, ".github/node_modules"),
  process.platform === "win32" ? "junction" : "dir",
);

export const files = {
  "Cargo.toml":
    '[workspace]\nmembers = ["crates/*"]\n[workspace.package]\nversion = "1.2.3"\n[workspace.dependencies]\nonetaskgraph-core = { path = "crates/onetaskgraph-core", version = "=1.2.3" }\n',
  "crates/onetaskgraph-core/Cargo.toml":
    '[package]\nname = "onetaskgraph-core"\nversion = "1.2.3"\n',
  "Cargo.lock":
    'version = 4\n[[package]]\nname = "onetaskgraph-core"\nversion = "1.2.3"\n[[package]]\nname = "external"\nversion = "1.2.3"\nsource = "registry+example"\nchecksum = "abc"\n',
  "pyproject.toml":
    '[project]\nname = "onetaskgraph-cli"\nversion = "1.2.3"\ndependencies = ["external==1.0.0"]\n',
  "sdks/python/pyproject.toml":
    '[project]\nname = "onetaskgraph-sdk"\nversion = "1.2.3"\ndependencies = ["onetaskgraph-cli==1.2.3"]\n',
  "sdks/python/src/onetaskgraph_sdk/__init__.py": '__version__ = "1.2.3"\nOTHER = 1\n',
  "sdks/typescript/src/index.ts": 'export const VERSION = "1.2.3";\nexport const OTHER = 1;\n',
  "sdks/typescript/package.json": JSON.stringify({
    name: "@onetaskgraph/sdk",
    version: "1.2.3",
    optionalDependencies: { "@onetaskgraph/cli": "1.2.3" },
  }),
  "npm/cli/package.json": JSON.stringify({
    name: "@onetaskgraph/cli",
    version: "1.2.3",
    optionalDependencies: Object.fromEntries(
      ["linux-x64", "linux-arm64", "darwin-arm64", "win32-x64"].map((x) => [
        `@onetaskgraph/cli-${x}`,
        "1.2.3",
      ]),
    ),
  }),
  "bun.lock": JSON.stringify(
    {
      lockfileVersion: 1,
      workspaces: {
        "sdks/typescript": {
          name: "@onetaskgraph/sdk",
          version: "1.2.3",
          optionalDependencies: { "@onetaskgraph/cli": "1.2.3" },
        },
      },
      packages: { external: ["external@1.0.0", "hash"] },
    },
    null,
    2,
  ).replace('"@onetaskgraph/cli": "1.2.3"', '"@onetaskgraph/cli": "1.2.3",'),
  "uv.lock":
    'version = 1\n[[package]]\nname = "onetaskgraph-cli"\nversion = "1.2.3"\nsource = { editable = "." }\n',
  "sdks/python/uv.lock":
    'version = 1\n[[package]]\nname = "onetaskgraph-sdk"\nversion = "1.2.3"\nsource = { virtual = "." }\ndependencies = [{name = "onetaskgraph-cli", version = "1.2.3"}]\n[package.metadata]\nrequires-dist = [{name = "onetaskgraph-cli", specifier = "==1.2.3"}]\n[[package]]\nname = "onetaskgraph-cli"\nversion = "1.2.3"\nsource = { editable = "../.." }\n',
  "crates/onetaskgraph-core/fixtures/Cargo.toml":
    '[package]\nname = "fixture"\nversion = "1.2.3"\n',
  "source.rs": "fn main() {}\n",
  "CHANGELOG.md": "# Changes\n",
};
for (const [p, s] of Object.entries(files)) put(p, s);
git("add", ".");
git("commit", "-qm", "base");
export const base = git("rev-parse", "HEAD");
export function revision(changes) {
  git("checkout", "-q", base);
  for (const [p, s] of Object.entries(changes)) {
    if (s === null) rmSync(resolve(history, p));
    else put(p, s);
  }
  git("add", ".");
  git("commit", "-qm", "scenario");
  return git("rev-parse", "HEAD");
}
export const versions = Object.fromEntries(
  Object.entries(files)
    .filter(([p]) => p !== "source.rs" && !p.includes("/fixtures/"))
    .map(([p, s]) => [
      p,
      p === "Cargo.lock"
        ? s.replace(
            'name = "onetaskgraph-core"\nversion = "1.2.3"',
            'name = "onetaskgraph-core"\nversion = "1.2.4"',
          )
        : s.replaceAll("1.2.3", "1.2.4"),
    ]),
);
export const heads = {
  version: revision(versions),
  source: revision({ "source.rs": 'fn main() { panic!("changed"); }\n' }),
  dependency: revision({
    "pyproject.toml": files["pyproject.toml"].replace("external==1.0.0", "external==2.0.0"),
  }),
};
export function event(ref = "feature", fork = false, author = "nickderobertis") {
  return {
    event_name: "pull_request",
    action: "opened",
    pull_request: {
      title: "chore: release v1.2.4",
      user: { login: author },
      head: {
        ref,
        repo: { full_name: fork ? "outside/repo" : "nickderobertis/onetaskgraph", fork },
      },
      base: { repo: { full_name: "nickderobertis/onetaskgraph" }, sha: base },
    },
  };
}
export function classify(
  payload,
  head = heads.version,
  baseRef = base,
  authoritativeEvent = "",
  extraEnv = {},
) {
  const path = resolve(history, "event.json");
  writeFileSync(path, typeof payload === "string" ? payload : JSON.stringify(payload));
  const output = resolve(history, "github-output");
  rmSync(output, { force: true });
  const result = spawnSync(
    process.execPath,
    [policy, "classify", "--event", path, "--base", baseRef, "--head", head],
    {
      cwd: history,
      encoding: "utf8",
      env: {
        ...process.env,
        ...extraEnv,
        GITHUB_EVENT_NAME: authoritativeEvent,
        GITHUB_OUTPUT: output,
      },
    },
  );
  assert.equal(result.status, 0, result.stderr);
  const decision = JSON.parse(result.stdout);
  assert.equal(
    readFileSync(output, "utf8"),
    `macosRequired=${decision.macosRequired}\nreason=${decision.reason}\n`,
  );
  return decision;
}
