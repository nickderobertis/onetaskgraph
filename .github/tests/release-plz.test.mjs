// The release-plz job's own steps, run as the workflow runs them: each `run` body is read out
// of .github/workflows/release-plz.yml and handed to bash with the environment its step
// declares. The guard decides from the two secrets; the auto-merge step talks to
// scripts/loopback-release-registries.py standing in for GitHub; and the release and
// preparation steps drive the real pinned release-plz over a scratch repository whose origin
// is a local bare repository and whose forge is that double, with every other HTTP route
// pointed at a closed port — so nothing reaches GitHub or a registry, and a journey proves
// what was planned rather than done.
import assert from "node:assert/strict";
import { execFileSync, spawn, spawnSync } from "node:child_process";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, before, test } from "node:test";
import { fileURLToPath } from "node:url";
import { evaluate, root, workflows } from "../workflow-graph.mjs";

const checkout = fileURLToPath(root);
const job = () => workflows()["release-plz.yml"].jobs["release-plz"];
const step = (name) => job().steps.find((s) => s.name === name);
const scratch = mkdtempSync(join(tmpdir(), "release-plz-job-"));
after(() => rmSync(scratch, { recursive: true, force: true }));

function runStep(definition, env, cwd = checkout) {
  const result = spawnSync("bash", ["-eo", "pipefail", "-c", definition.run], {
    cwd,
    encoding: "utf8",
    env: { ...process.env, ...env },
  });
  return { status: result.status, output: `${result.stdout}${result.stderr}` };
}

test("the guard enables the job exactly when both release secrets are present, as the guard job did", () => {
  const guard = job().steps[0];
  assert.equal(guard.id, "guard");
  const cases = [
    ["both secrets present", "release-token", "cargo-token", "true"],
    ["RELEASE_PLZ_TOKEN absent, as in a fork", "", "cargo-token", "false"],
    ["CARGO_REGISTRY_TOKEN absent", "release-token", "", "false"],
    ["neither secret present", "", "", "false"],
  ];
  for (const [name, release, cargo, expected] of cases) {
    const output = join(scratch, `guard-${expected}-${release ? 1 : 0}${cargo ? 1 : 0}`);
    writeFileSync(output, "");
    const result = runStep(guard, {
      RELEASE_TOKEN: release,
      CARGO_TOKEN: cargo,
      GITHUB_OUTPUT: output,
    });
    assert.equal(result.status, 0, name);
    assert.equal(readFileSync(output, "utf8"), `enabled=${expected}\n`, name);
    // Every later step reads the guard's answer, so a disabled job does nothing at all.
    for (const later of job().steps.slice(1))
      assert.equal(
        evaluate(later.if, { steps: { guard: { outputs: { enabled: expected } } } }),
        expected === "true",
        `${name}: ${later.name || later.uses}`,
      );
  }
});

let double;
let ports;
before(async () => {
  const state = join(scratch, "github");
  mkdirSync(state);
  double = spawn("python3", ["scripts/loopback-release-registries.py", state], {
    cwd: checkout,
    stdio: ["ignore", "ignore", "inherit"],
  });
  for (let waited = 0; !existsSync(join(state, "ports.json")); waited += 100) {
    assert.ok(waited < 30000, "the GitHub double never reported its ports");
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  ports = JSON.parse(readFileSync(join(state, "ports.json"), "utf8"));
});
after(() => double?.kill());

const repository = "nickderobertis/onetaskgraph";
const pull = (number, ref, owner = repository) => ({
  number,
  node_id: `PR_${number}`,
  head: { ref, repo: { full_name: owner } },
});
function automerge(pulls, refuse = [], answers = {}, env = {}) {
  const state = join(scratch, "github");
  writeFileSync(join(state, "control.json"), JSON.stringify({ pulls, refuse, ...answers }));
  const before = existsSync(join(state, "requests.jsonl"))
    ? readFileSync(join(state, "requests.jsonl"), "utf8")
    : "";
  const result = runStep(step("Enable auto-merge"), {
    GH_TOKEN: "fixture-token",
    GITHUB_REPOSITORY: repository,
    GITHUB_API_URL: `http://127.0.0.1:${ports.main}/github/api`,
    GITHUB_GRAPHQL_URL: `http://127.0.0.1:${ports.main}/github/api/graphql`,
    ...env,
  });
  const sent = readFileSync(join(state, "requests.jsonl"), "utf8")
    .slice(before.length)
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line));
  return { ...result, sent };
}

test("the auto-merge step asks GitHub to merge release-plz's pull request once its checks pass, and no other", () => {
  const result = automerge([
    pull(41, "feature/unrelated"),
    pull(42, "release-plz-2026-10-01", "someone/fork"),
    pull(39, "release-plz-0.7.9"),
    pull(43, "release-plz-0.8.0"),
    pull(44, "renovate/release-plz-action"),
  ]);
  assert.equal(result.status, 0, result.output);
  const asked = result.sent.filter((r) => r.action === "graphql");
  assert.equal(asked.length, 1, JSON.stringify(result.sent));
  assert.equal(asked[0].pull_request_id, "PR_43");
  // Auto-merge, conditional on the required checks, rather than a merge: the mutation that
  // waits for them, with the squash this repository merges by.
  assert.match(asked[0].query, /enablePullRequestAutoMerge/);
  assert.match(asked[0].query, /mergeMethod: SQUASH/);
  assert.doesNotMatch(asked[0].query, /mergePullRequest\b/);
  assert.ok(!result.sent.some((r) => /\/merge\b/.test(r.path)), "a direct merge was requested");
  assert.match(
    result.output,
    /#43 \(release-plz-0\.8\.0\) will squash-merge once its required checks pass/,
  );

  const none = automerge([
    pull(41, "feature/unrelated"),
    pull(42, "release-plz-x", "someone/fork"),
  ]);
  assert.equal(none.status, 0, none.output);
  assert.deepEqual(
    none.sent.filter((r) => r.action === "graphql"),
    [],
  );
});

test("the auto-merge step follows a redirect within GitHub's own origin, credentials and all", () => {
  const result = automerge([pull(43, "release-plz-0.8.0")], [], { redirect_api_same_origin: true });
  assert.equal(result.status, 0, result.output);
  const listed = result.sent.findIndex((r) => r.action === "redirected-same-origin");
  assert.ok(listed >= 0, JSON.stringify(result.sent));
  const followed = result.sent[listed + 1];
  assert.equal(followed.action, "list-pulls");
  assert.match(followed.path, /redirected=1/);
  assert.equal(followed.port, ports.main);
  assert.ok(followed.authorized, "the token was dropped on a same-origin redirect");
  assert.equal(result.sent.filter((r) => r.action === "graphql").length, 1);
  assert.match(result.output, /#43 \(release-plz-0\.8\.0\) will squash-merge/);
});

test("the auto-merge step fails visibly when GitHub refuses it or cannot be asked", () => {
  const refused = automerge(
    [pull(43, "release-plz-0.8.0")],
    [{ registry: "github", action: "automerge" }],
  );
  assert.equal(refused.status, 1);
  assert.match(refused.output, /GitHub refused auto-merge for #43/);
  assert.match(refused.output, /not in the correct state to enable auto-merge/);
  assert.match(refused.output, /next: the release pull request will not merge itself/);
  const unreachable = automerge(
    [pull(43, "release-plz-0.8.0")],
    [{ registry: "github", action: "list-pulls" }],
  );
  assert.equal(unreachable.status, 1);
  assert.match(unreachable.output, /HTTP 502/);
  assert.equal(unreachable.sent.filter((r) => r.action === "graphql").length, 0);
  const malformed = automerge([pull(43, "release-plz-0.8.0"), 7]);
  assert.equal(malformed.status, 1);
  assert.match(malformed.output, /GitHub listed something that is not a pull request: 7/);
  assert.equal(malformed.sent.filter((r) => r.action === "graphql").length, 0);
  // A number GitHub could not have assigned — a boolean, zero, a negative — is no pull request.
  for (const number of [true, 0, -43]) {
    const bad = automerge([{ ...pull(43, "release-plz-0.8.0"), number }]);
    assert.equal(bad.status, 1, `${number}: ${bad.output}`);
    assert.match(bad.output, /GitHub listed something that is not a pull request/);
    assert.equal(bad.sent.filter((r) => r.action === "graphql").length, 0);
  }
  const unset = automerge([pull(43, "release-plz-0.8.0")], [], {
    automerge_answer: { data: { enablePullRequestAutoMerge: { pullRequest: { number: 43 } } } },
  });
  assert.equal(unset.status, 1);
  assert.match(unset.output, /without a squash auto-merge request on it/);
  assert.match(unset.output, /next: the release pull request will not merge itself/);
  // Its own inputs, and every answer GitHub could give that is not a list of pull requests
  // or a squash auto-merge request, end the step red with a next action.
  const release = [pull(43, "release-plz-0.8.0")];
  for (const [name, refused, answers, env, reason] of [
    [
      "no token",
      [],
      {},
      { GH_TOKEN: "" },
      /GH_TOKEN and an owner\/name GITHUB_REPOSITORY are required/,
    ],
    [
      "a repository that is not owner/name",
      [],
      {},
      { GITHUB_REPOSITORY: "onetaskgraph" },
      /owner\/name GITHUB_REPOSITORY/,
    ],
    [
      "an API that is not http(s)",
      [],
      {},
      { GITHUB_API_URL: "ftp://example.invalid" },
      /GITHUB_API_URL is 'ftp:\/\/example\.invalid', not an http\(s\) URL/,
    ],
    [
      "a redirect to another origin",
      [],
      { redirect_api: true },
      {},
      /refusing to follow a redirect to http:\/\/127\.0\.0\.1:\d+ with credentials/,
    ],
    [
      "an API URL that cannot be parsed",
      [],
      {},
      { GITHUB_API_URL: "http://[broken" },
      /GITHUB_API_URL is 'http:\/\/\[broken', not an http\(s\) URL/,
    ],
    [
      "a pull list that is not a list",
      [],
      { pulls_raw: { message: "moved" } },
      {},
      /open pull requests is not a list/,
    ],
    ["a pull list that is not JSON", [], { pulls_raw: "<html>" }, {}, /could not ask GitHub/],
    [
      "a GraphQL answer that is not an envelope",
      [],
      { automerge_answer: { data: [] } },
      {},
      /enable-release-automerge: /,
    ],
  ]) {
    const result = automerge(release, refused, answers, env);
    assert.equal(result.status, 1, `${name}: ${result.output}`);
    assert.match(result.output, reason, name);
    assert.match(result.output, /enable-release-automerge: next: /, name);
    // The token never reaches an origin other than the API it was given for.
    assert.ok(!result.sent.some((r) => r.port === ports.checksums && r.authorized), name);
  }
});

// The pinned release-plz, which `just bootstrap` provisions where every script resolves it.
const pinned = spawnSync("bash", ["scripts/scoped-release-plz.sh", "resolve"], {
  cwd: checkout,
  encoding: "utf8",
});
const skipPreparation =
  process.platform === "win32"
    ? "release preparation runs only on ubuntu-latest, and Git rewrites the fixture's GitHub-shaped origin before release-plz reads it on Windows (scripts/check-real-release-preparation.sh says why)"
    : false;

test("on an eligible push the job prepares the version bump, and on the release PR's merge it plans the tag and the GitHub Release, writing no remote or registry", {
  skip: skipPreparation,
}, () => {
  assert.equal(
    pinned.status,
    0,
    `the pinned release-plz is not provisioned: ${pinned.stderr}; run 'just bootstrap'`,
  );
  const fixture = join(scratch, "fixture");
  mkdirSync(fixture);
  // A released baseline of the tree under test, tagged as every package's release, with an
  // origin that is a local bare repository behind a GitHub-shaped URL.
  const baseline = execFileSync(
    "bash",
    [
      "-c",
      `set -euo pipefail
ROOT="$1"; S="$2"
source "$ROOT/scripts/scratch-clone.sh"
scratch_clone_strip_git_env
repo="$S/repo"
scratch_clone "$ROOT" "$repo" >/dev/null
git -C "$repo" checkout --quiet --orphan fixture-main
mkdir -p "$S/hooks" "$S/registry" "$S/state"
git -C "$repo" config core.hooksPath "$S/hooks"
# The tree under review exactly: nothing HEAD has that the working tree deleted.
git -C "$repo" rm -rq --cached .
find "$repo" -mindepth 1 -maxdepth 1 ! -name .git -exec rm -rf {} +
copy_tracked_files "$ROOT" "$repo"
perl -pi -e 's/^semver_check = true$/semver_check = false/' "$repo/release-plz.toml"
version=$(sed -n 's/^version = "\\([^"]*\\)"/\\1/p' "$repo/crates/onetaskgraph/Cargo.toml" | head -n1)
git -C "$repo" add -A
git -C "$repo" -c user.name=fixture -c user.email=fixture@example.invalid commit --quiet -m "chore: release v$version"
git -C "$repo" for-each-ref --format='delete %(refname)' refs/tags | git -C "$repo" update-ref --stdin
git -C "$repo" tag "v$version"
for crate in $(cd "$repo" && cargo metadata --no-deps --format-version 1 | python3 -c 'import json, sys; print(" ".join(p["name"] for p in json.load(sys.stdin)["packages"]))'); do
  [ "$crate" = onetaskgraph ] || git -C "$repo" tag "$crate-v$version"
done
git -C "$repo" archive HEAD | extract_tar_stream "$S/registry"
git init --quiet --bare "$S/origin.git"
git -C "$repo" config "url.$S/origin.git.insteadOf" https://github.com/check/onetaskgraph-release-fixture.git
git -C "$repo" remote set-url origin https://github.com/check/onetaskgraph-release-fixture.git
git -C "$repo" push --quiet --set-upstream origin HEAD --tags
git --git-dir="$S/origin.git" symbolic-ref HEAD refs/heads/fixture-main
git -C "$repo" remote set-head origin fixture-main
printf '%s' "$version"`,
      "fixture",
      checkout,
      fixture,
    ],
    { encoding: "utf8" },
  );
  const repo = join(fixture, "repo");
  const state = join(fixture, "state");
  // The scoped tool location every script resolves release-plz from, holding a launcher that
  // hands each subcommand to the real pinned binary — except what would write: `release` is
  // run as its own dry run against a repository URL with no forge behind it, and `release-pr`
  // records the tree it was handed instead of pushing a branch and opening a pull request.
  const pin = execFileSync("bash", ["scripts/scoped-release-plz.sh", "pin"], {
    cwd: checkout,
    encoding: "utf8",
  }).trim();
  const launcher = join(fixture, "tools/release-plz", pin, "bin/release-plz");
  mkdirSync(join(launcher, ".."), { recursive: true });
  writeFileSync(
    launcher,
    `#!/usr/bin/env bash
set -euo pipefail
printf '%s\\n' "$*" >> "$FIXTURE_STATE/release-plz.log"
case "\${1:-}" in
  update) exec "$REAL_RELEASE_PLZ" "$@" --forge github --registry-manifest-path "$FIXTURE_REGISTRY/Cargo.toml" ;;
  release) exec "$REAL_RELEASE_PLZ" "$@" --dry-run --forge github --repo-url "$FIXTURE_REPO_URL" ;;
  release-pr)
    sed -n 's/^version = "\\([^"]*\\)"/\\1/p' crates/onetaskgraph/Cargo.toml | head -n1 > "$FIXTURE_STATE/proposed"
    scripts/set-version.sh --check > "$FIXTURE_STATE/proposed-check" 2>&1
    exit 0 ;;
  *) exec "$REAL_RELEASE_PLZ" "$@" ;;
esac
`,
  );
  chmodSync(launcher, 0o755);
  const env = {
    ONETASKGRAPH_TOOLS_HOME: join(fixture, "tools"),
    REAL_RELEASE_PLZ: pinned.stdout.trim(),
    FIXTURE_STATE: state,
    FIXTURE_REGISTRY: join(fixture, "registry"),
    // A repository URL on the GitHub double, which answers release-plz's one forge read.
    FIXTURE_REPO_URL: `http://127.0.0.1:${ports.main}/check/onetaskgraph-release-fixture`,
    GITHUB_REF_NAME: "fixture-main",
    GITHUB_TOKEN: "fixture-token",
    GIT_TOKEN: "fixture-token",
    CARGO_REGISTRY_TOKEN: "",
    UV_OFFLINE: "1",
    // Every way out of the fixture but the double on loopback is a refused connection, so a
    // step that reached GitHub or a registry would fail here rather than act.
    HTTP_PROXY: "http://127.0.0.1:9",
    HTTPS_PROXY: "http://127.0.0.1:9",
    http_proxy: "http://127.0.0.1:9",
    https_proxy: "http://127.0.0.1:9",
    NO_PROXY: "127.0.0.1",
    no_proxy: "127.0.0.1",
  };
  const forgeWrites = () =>
    readFileSync(join(scratch, "github", "requests.jsonl"), "utf8")
      .split("\n")
      .filter((line) => line.includes('"forge-write"'));
  const git = (...args) => execFileSync("git", ["-C", repo, ...args], { encoding: "utf8" });
  const remote = () =>
    execFileSync("git", ["--git-dir", join(fixture, "origin.git"), "for-each-ref"], {
      encoding: "utf8",
    });
  const release = step("Tag and cut the GitHub Release with the PAT");
  const prepare = step("Open or update the release pull request");
  const bumped = baseline.replace(
    /^(\d+)\.(\d+)\.(\d+)$/,
    (_, major, minor, patch) => `${major}.${minor}.${Number(patch) + 1}`,
  );

  // An eligible push: a fix to a published crate.
  writeFileSync(
    join(repo, "crates/onetaskgraph-core/src/lib.rs"),
    `${readFileSync(join(repo, "crates/onetaskgraph-core/src/lib.rs"), "utf8")}// fixture change\n`,
  );
  git(
    "-c",
    "user.name=fixture",
    "-c",
    "user.email=fixture@example.invalid",
    "commit",
    "--quiet",
    "-am",
    "fix(core): repair a fixture defect",
  );
  git("push", "--quiet", "origin", "HEAD:fixture-main");
  let published = remote();
  let ran = runStep(release, env, repo);
  assert.equal(ran.status, 0, ran.output);
  assert.doesNotMatch(
    ran.output,
    /creation of tag/,
    "a push with nothing unreleased planned a tag",
  );
  ran = runStep(prepare, env, repo);
  assert.equal(ran.status, 0, ran.output);
  assert.equal(
    readFileSync(join(state, "proposed"), "utf8").trim(),
    bumped,
    "the version bump release-plz prepared",
  );
  assert.equal(
    readFileSync(join(state, "proposed-check"), "utf8"),
    "",
    "every manifest agreed when the pull request was proposed",
  );
  assert.match(
    ran.output,
    new RegExp(`proposed the package release pull request for ${bumped.replaceAll(".", "\\.")}`),
  );
  assert.equal(remote(), published, "the preparation wrote to the remote");

  // The release pull request's merge: the bump lands on the branch as one commit.
  git(
    "-c",
    "user.name=fixture",
    "-c",
    "user.email=fixture@example.invalid",
    "commit",
    "--quiet",
    "-am",
    `chore: release v${bumped} (#9999)`,
  );
  git("push", "--quiet", "origin", "HEAD:fixture-main");
  published = remote();
  ran = runStep(release, env, repo);
  assert.equal(ran.status, 0, ran.output);
  assert.match(ran.output, new RegExp(`creation of tag 'v${bumped.replaceAll(".", "\\.")}'`));
  assert.match(ran.output, /creation of git release/);
  for (const crate of ["onetaskgraph-core", "onetaskgraph-plugin-api"])
    assert.match(
      ran.output,
      new RegExp(`creation of tag '${crate}-v${bumped.replaceAll(".", "\\.")}'`),
    );
  assert.equal(remote(), published, "the release wrote a tag to the remote");
  assert.equal(git("tag", "--list", `v${bumped}`), "", "the release wrote a tag locally");
  assert.deepEqual(forgeWrites(), [], "a step wrote to the forge");
});
