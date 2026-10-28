// The release's GitHub calls and the double that stands GitHub in, held to
// config/github-interfaces.toml by .github/tests/github_interfaces.py. Each drift
// below is written into a copy of the pin, so the gate is watched refusing it
// rather than trusted.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../..", import.meta.url));
const pinned = readFileSync(join(root, "config/github-interfaces.toml"), "utf8");

function gate(pin) {
  const args = [".github/tests/github_interfaces.py"];
  let scratch;
  if (pin !== undefined) {
    scratch = mkdtempSync(join(tmpdir(), "github-interfaces-"));
    args.push(join(scratch, "pin.toml"));
    writeFileSync(args[1], pin);
  }
  try {
    return spawnSync("python3", args, { cwd: root, encoding: "utf8" });
  } finally {
    if (scratch) rmSync(scratch, { recursive: true, force: true });
  }
}

function drifted(from, to) {
  assert.ok(pinned.includes(from), `the pin no longer holds ${from}`);
  return pinned.replace(from, to);
}

test("the release's calls and the double agree with the pinned interface", () => {
  const result = gate();
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stderr, "");
});

const drifts = [
  [
    "a field the schema slice lacks",
    drifted('number = "Int"\n', ""),
    "graphql: PullRequest has no field number in the pinned schema slice",
  ],
  [
    "an input field the schema slice lacks",
    drifted('mergeMethod = "PullRequestMergeMethod"\nauthorEmail', "authorEmail"),
    "graphql: EnablePullRequestAutoMergeInput has no field mergeMethod",
  ],
  [
    "an enum value the schema slice lacks",
    drifted('["MERGE", "SQUASH", "REBASE"]', '["MERGE", "REBASE"]'),
    "graphql: SQUASH is not a value of PullRequestMergeMethod",
  ],
  [
    "a document other than the one the step sends",
    drifted("number autoMergeRequest {", "autoMergeRequest {"),
    "graphql: scripts/enable-release-automerge.py sends a document other than the pinned one",
  ],
  [
    "a read the answer does not carry",
    drifted('"state": "uploaded",', ""),
    "release-by-tag: `reads` names assets[].state, which GitHub's pinned answer does not carry",
  ],
  [
    "a read the parser does not make",
    drifted(
      'reads = ["id", "name", "size", "state"]',
      'reads = ["id", "name", "size", "state", "label"]',
    ),
    "upload-asset: removing label from GitHub's answer leaves the parser's result unchanged",
  ],
  [
    "a pull request GitHub answers without its number",
    drifted('    "number": 3647,\n', ""),
    "open-pulls: the release's parser refuses GitHub's pinned answer",
  ],
  [
    "a field the double answers that GitHub does not",
    drifted('  "id": 414082970,\n', ""),
    "release-by-tag: scripts/loopback-release-registries.py answers with id, which GitHub's pinned answer does not carry",
  ],
  [
    "a read the parser makes that the pin does not name",
    drifted('reads = ["id", "name", "size", "state"]', 'reads = ["id", "size", "state"]'),
    "upload-asset: the parser reads name, which `reads` does not name",
  ],
  [
    "a path other than the one a script sends",
    drifted(
      'path = "/repos/{owner}/{repo}/pulls?state=open&per_page=100"',
      'path = "/repos/{owner}/{repo}/pulls?state=all"',
    ),
    "a release script sent GET /repos/nickderobertis/onetaskgraph/pulls?state=open&per_page=100, which the pin describes as no call of GitHub's",
  ],
  [
    "a method other than the one a script sends",
    drifted('method = "DELETE"', 'method = "POST"'),
    "a release script sent DELETE /repos/nickderobertis/onetaskgraph/releases/assets/644642357, which the pin describes as no call of GitHub's",
  ],
  [
    "a status other than the one the double answers",
    drifted("status = 204", "status = 200"),
    "delete-asset: scripts/loopback-release-registries.py answers HTTP 204, where GitHub answers 200",
  ],
  [
    "an API host other than the one the scripts default to",
    drifted(
      'host = "api.github.com"\nmethod = "GET"\npath = "/repos/{owner}/{repo}/releases/tags/{tag}"',
      'host = "elsewhere.github.com"\nmethod = "GET"\npath = "/repos/{owner}/{repo}/releases/tags/{tag}"',
    ),
    "release-by-tag: scripts/release_cd.py sends it to api.github.com, where the pin says elsewhere.github.com",
  ],
  [
    "an upload host other than the one the pinned release names",
    drifted('host = "uploads.github.com"', 'host = "api.github.com"'),
    "upload-asset: the pinned release names 'https://uploads.github.com/repos/nickderobertis/onetaskgraph/releases/414082970/assets{?name,label}' to upload to, where the pin says api.github.com",
  ],
];

test("the gate refuses a tag that is not a release's", () => {
  const result = gate(drifted('tag = "v0.7.1"', 'tag = "../v0.7.1"'));
  assert.equal(result.status, 1, result.stderr);
  assert.ok(result.stderr.includes("github-interfaces: release-by-tag: `tag` is '../v0.7.1'"));
});

test("the gate refuses a pinned release with no asset to take back", () => {
  const result = gate(drifted('"assets": [\n', '"assets": [],\n  "listed": [\n'));
  assert.equal(result.status, 1, result.stderr);
  assert.ok(
    result.stderr.includes(
      "github-interfaces: release-by-tag: `answer` must be a release with an upload_url and at least one named asset",
    ),
    result.stderr,
  );
});

test("the gate refuses a pin it cannot read, before reading anything else", () => {
  const result = gate(drifted("status = 201", 'status = "201"'));
  assert.equal(result.status, 1, result.stderr);
  assert.ok(
    result.stderr.includes("github-interfaces: upload-asset: `status` must be of type int"),
  );
  assert.ok(result.stderr.includes("github-interfaces: next: bring config/github-interfaces.toml"));
});

for (const [name, pin, message] of drifts) {
  test(`the gate refuses ${name}`, () => {
    const result = gate(pin);
    assert.equal(result.status, 1, result.stderr);
    assert.ok(result.stderr.includes(`github-interfaces: ${message}`), result.stderr);
    assert.ok(result.stderr.includes("github-interfaces: next: "), result.stderr);
  });
}
