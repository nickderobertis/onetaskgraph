import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readdirSync, readFileSync } from "node:fs";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { graph } from "../workflow-graph.mjs";
import { definitions } from "./workflow-definitions.mjs";

// Which jobs run where, as the run graph resolves the workflow files (workflow-graph.mjs). A
// pull request's last commit runs exactly the four jobs branch protection requires, because
// every other job spends a slot of the account-wide concurrency limit (onetaskgraph-ci-job-count);
// a push to main runs release-plz, the Rust cache warmer that catches a main that no longer
// builds, and the canonical gallery, which skips release-plz's own release commit. The release
// tail at the end of publish reads GitHub with the job's own token, granted read alone.

const root = fileURLToPath(new URL("../../", import.meta.url));
const REQUIRED = ["lint", "test-linux", "test-windows", "test-macos"];
// A release commit and an ordinary push of this repository, read from its own history.
const RELEASE_COMMIT = "3b6b228f168ab17c94613a831e1eec96a225d949";
const ORDINARY_PUSH = "e2d3bf08e0ced82120f38a708dc2b7a92d26ce7f";
// The workflows whose triggers and jobs no CI budget governs, held byte for byte: a job or a
// trigger added to one of them would spend the slots the budgets above count without either
// test noticing. Moving one means re-deciding its schedule, then this digest.
const UNCHANGED = {
  "linear-documents.yml": "cbee6e4eaa87bb4231469d90995eab95398bd1eb48885088a14dc405dc08088d",
  "live-janitor.yml": "12f972eff5e7402be7c9d87de5d5a01191ef96499605d01bc7f4f957001feebb",
  "release-plz.yml": "a1a98448486d5c0ff89880124f5d0757d0dfb1cf8317ca945b02e6af00d5469e",
  "rust-cache-warm.yml": "9baa87baacb5eacd55e2744152d45ecb40f9f354b9944c27c358b158a80786ce",
};
const WORKFLOWS = ["ci.yml", "release.yml", "visual-docs.yml", ...Object.keys(UNCHANGED)];

const git = (...args) => execFileSync("git", args, { cwd: root, encoding: "utf8" }).trim();
const push = (sha) => ({
  event: "push",
  branch: "main",
  revisions: { cwd: root, before: git("rev-parse", `${sha}^`), after: sha },
  headCommitMessage: git("log", "-1", "--format=%B", sha),
});
const running = (runs) =>
  Object.fromEntries(runs.filter((run) => run.rows.length).map((run) => [run.workflow, run.rows]));

export function problems(all, texts = {}) {
  const found = [];
  const names = Object.keys(all).sort();
  if (JSON.stringify(names) !== JSON.stringify([...WORKFLOWS].sort()))
    found.push(`the workflows are ${names.join(", ")}`);
  for (const [name, digest] of Object.entries(UNCHANGED))
    if (createHash("sha256").update(texts[name]).digest("hex") !== digest)
      found.push(`${name} changed, and this change does not name it`);
  // A pull request's last commit, from this repository, from a fork, and from release-plz's
  // own branch, on every action that starts a pull request's checks.
  for (const action of ["opened", "synchronize", "reopened"])
    for (const [headRef, fork] of [
      ["feature", false],
      ["feature", true],
      ["release-plz-2026-10-31", false],
    ]) {
      const scenario = { event: "pull_request", action, headRef, fork };
      const runs = running(graph(scenario, { macosRequired: true, reason: "a change" }, all));
      for (const [workflow, rows] of Object.entries(runs))
        if (workflow !== "ci.yml")
          found.push(
            `${workflow} runs ${rows.map((r) => r.name).join(", ")} on pull_request ${action}`,
          );
      const jobs = (runs["ci.yml"] ?? []).map((row) => row.name);
      const expected =
        headRef.startsWith("release-plz-") && !fork ? REQUIRED.slice(0, 3) : REQUIRED;
      if (JSON.stringify(jobs) !== JSON.stringify(expected))
        found.push(
          `a pull request ${action} from ${headRef}${fork ? " (fork)" : ""} runs ${jobs.join(", ")}`,
        );
    }
  const release = running(graph(push(RELEASE_COMMIT), { macosRequired: false }, all));
  for (const workflow of Object.keys(release))
    if (!["release-plz.yml", "rust-cache-warm.yml"].includes(workflow))
      found.push(`${workflow} runs on the release commit`);
  if (!release["release-plz.yml"]) found.push("release-plz.yml does not run on the release commit");
  const ordinary = running(graph(push(ORDINARY_PUSH), { macosRequired: false }, all));
  if (Object.keys(ordinary).sort().join() !== "release-plz.yml,rust-cache-warm.yml,visual-docs.yml")
    found.push(`a push to main runs ${Object.keys(ordinary).sort().join(", ")}`);
  // The release tail's token: read-only on what it reads, and handed to its step.
  const publish = all["release.yml"]?.jobs?.publish;
  for (const [scope, level] of Object.entries({
    actions: "read",
    contents: "read",
    "pull-requests": "read",
  }))
    if (publish?.permissions?.[scope] !== level)
      found.push(`publish does not grant ${scope}: ${level}`);
  if (publish?.steps?.at(-1)?.env?.GH_TOKEN !== `\${{ github.token }}`)
    found.push("publish's last step is not handed the job's token");
  const visual = all["visual-docs.yml"]?.on ?? {};
  if (JSON.stringify(visual.pull_request?.types) !== '["closed"]')
    found.push("visual-docs.yml no longer runs on a closed pull request alone");
  if (!visual.schedule?.length) found.push("visual-docs.yml lost its schedule");
  if (!("workflow_dispatch" in visual)) found.push("visual-docs.yml lost its workflow_dispatch");
  if (JSON.stringify(visual.push?.branches) !== '["main"]')
    found.push("visual-docs.yml no longer publishes on a push to main");
  return found;
}
const texts = () =>
  Object.fromEntries(
    readdirSync(new URL("../workflows/", import.meta.url)).map((name) => [
      name,
      readFileSync(new URL(`../workflows/${name}`, import.meta.url), "utf8"),
    ]),
  );

test("a pull request runs the four jobs, a push to main runs release-plz, the warmer and the gallery", () => {
  assert.deepEqual(problems(definitions(), texts()), []);
});

test("the release commit runs release-plz and the warmer, and visual-docs skips it", () => {
  const runs = graph(push(RELEASE_COMMIT), { macosRequired: false });
  assert.match(push(RELEASE_COMMIT).headCommitMessage, /^chore: release v\d+\.\d+\.\d+/);
  assert.deepEqual(runs.find((run) => run.workflow === "visual-docs.yml").rows, []);
  assert.deepEqual(
    running(runs)["release-plz.yml"].map((row) => row.name),
    ["release-plz"],
  );
});

test("another job or workflow on a pull request or on the release commit is refused", () => {
  const cases = {
    "notignored.yml runs": (all) => {
      all["notignored.yml"] = {
        name: "notignored",
        on: { pull_request: null },
        jobs: { suppressions: { "runs-on": "ubuntu-latest", steps: [{ run: "true" }] } },
      };
    },
    "runs lint, test-linux, test-windows, test-macos, deny": (all) => {
      all["ci.yml"].jobs.deny = { "runs-on": "ubuntu-latest", steps: [{ run: "just deny" }] };
    },
    "visual-docs.yml runs": (all) => {
      all["visual-docs.yml"].on.pull_request.types.push("synchronize");
    },
    "ci.yml runs on the release commit": (all) => {
      all["ci.yml"].on.push = { branches: ["main"] };
      all["ci.yml"].jobs.lint.if = "true";
    },
    "visual-docs.yml runs on the release commit": (all) => {
      delete all["visual-docs.yml"].jobs["visual-docs"].if;
    },
    "release.yml runs on the release commit": (all) => {
      all["release.yml"].on = { push: { branches: ["main"] }, release: { types: ["published"] } };
    },
    "lost its schedule": (all) => {
      delete all["visual-docs.yml"].on.schedule;
    },
    "lost its workflow_dispatch": (all) => {
      delete all["visual-docs.yml"].on.workflow_dispatch;
    },
    "publish does not grant actions: read": (all) => {
      delete all["release.yml"].jobs.publish.permissions.actions;
    },
    "publish does not grant contents: read": (all) => {
      all["release.yml"].jobs.publish.permissions.contents = "write";
    },
    "publish does not grant pull-requests: read": (all) => {
      delete all["release.yml"].jobs.publish.permissions["pull-requests"];
    },
    "not handed the job's token": (all) => {
      delete all["release.yml"].jobs.publish.steps.at(-1).env;
    },
    "closed pull request alone": (all) => {
      delete all["visual-docs.yml"].on.pull_request;
    },
  };
  for (const [expected, change] of Object.entries(cases)) {
    const all = definitions();
    change(all);
    const found = problems(all, texts());
    assert.ok(
      found.some((problem) => problem.includes(expected)),
      `${expected}: ${JSON.stringify(found)}`,
    );
  }
});

test("a workflow this change does not name is held byte for byte", () => {
  for (const name of Object.keys(UNCHANGED)) {
    const changed = texts();
    changed[name] += "\n# an edit\n";
    assert.ok(
      problems(definitions(), changed).includes(
        `${name} changed, and this change does not name it`,
      ),
      name,
    );
  }
});
