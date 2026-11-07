import "../test-setup.mjs";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { readdirSync, readFileSync } from "node:fs";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { parse } from "yaml";
import { graph } from "../workflow-graph.mjs";
import { definitions } from "./workflow-definitions.mjs";

// Which jobs run where, as the run graph resolves the workflow files (workflow-graph.mjs). A
// pull request's last commit runs lint, test-linux, test-windows and budgets, and test-macos on
// every pull request, including release-plz’s own branch. Every other job spends a slot of the
// account-wide concurrency limit (onetaskgraph-ci-job-count);
// a push to main runs release-plz, ci.yml's budgets job alone, the Rust cache warmer that catches a
// main that no longer builds, and the canonical gallery, which skips release-plz's own release commit. The release
// tail at the end of publish reads GitHub with the job's own token, granted read alone.

const root = fileURLToPath(new URL("../../", import.meta.url));
const REQUIRED = ["lint", "test-linux", "test-windows", "test-macos", "budgets"];
// A release commit and an ordinary push of this repository, read from its own history.
const RELEASE_COMMIT = "3b6b228f168ab17c94613a831e1eec96a225d949";
const ORDINARY_PUSH = "e2d3bf08e0ced82120f38a708dc2b7a92d26ce7f";
// The workflows whose triggers and jobs no CI budget governs, held byte for byte: a job or a
// trigger added to one of them would spend the slots the budgets above count without either
// test noticing. Moving one means re-deciding its schedule, then this digest.
const UNCHANGED = {
  "release-plz.yml": "009bb4f705c0d9ce349f887d92b47de09059897ccbce07f166f1021fcec36183",
  "linear-documents.yml": "4c00908a0f0e78dd5927e17d989ef54440eec1f0237317755c4c241a3894a1ab",
  "live-janitor.yml": "12f972eff5e7402be7c9d87de5d5a01191ef96499605d01bc7f4f957001feebb",
  "rust-cache-warm.yml": "2592cb8dba1e30f4741304b3d59e711cf895bec22073233b2683cdbba0bb412c",
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
  // A pull request's last commit, from this repository, from a fork, from release-plz's own
  // branch, and from a fork's branch of that name, on every action that starts a pull request's
  // checks: lint, test-linux and test-windows always, and test-macos on every branch.
  for (const action of ["opened", "synchronize", "reopened"])
    for (const [headRef, fork] of [
      ["feature", false],
      ["feature", true],
      ["release-plz-2026-10-31", false],
      ["release-plz-2026-10-31", true],
    ]) {
      const scenario = { event: "pull_request", action, headRef, fork };
      const runs = running(graph(scenario, { macosRequired: true, reason: "a change" }, all));
      for (const [workflow, rows] of Object.entries(runs))
        if (workflow !== "ci.yml")
          found.push(
            `${workflow} runs ${rows.map((r) => r.name).join(", ")} on pull_request ${action}`,
          );
      const jobs = (runs["ci.yml"] ?? []).map((row) => row.name);
      const expected = REQUIRED;
      if (JSON.stringify(jobs) !== JSON.stringify(expected))
        found.push(
          `a pull request ${action} from ${headRef}${fork ? " (fork)" : ""} runs ${jobs.join(", ")}`,
        );
    }
  const release = running(graph(push(RELEASE_COMMIT), { macosRequired: false }, all));
  for (const workflow of Object.keys(release))
    if (!["release-plz.yml", "rust-cache-warm.yml", "ci.yml"].includes(workflow))
      found.push(`${workflow} runs on the release commit`);
  if (!release["release-plz.yml"]) found.push("release-plz.yml does not run on the release commit");
  const ordinary = running(graph(push(ORDINARY_PUSH), { macosRequired: false }, all));
  if (
    Object.keys(ordinary).sort().join() !==
    "ci.yml,release-plz.yml,rust-cache-warm.yml,visual-docs.yml"
  )
    found.push(`a push to main runs ${Object.keys(ordinary).sort().join(", ")}`);
  for (const runs of [release, ordinary])
    if ((runs["ci.yml"] ?? []).map((row) => row.name).join() !== "budgets")
      found.push(
        `a push to main runs ci.yml's ${(runs["ci.yml"] ?? []).map((r) => r.name).join()}`,
      );
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

test("every pull request runs five jobs; main pushes run release tooling, budgets, the warmer and the gallery", () => {
  assert.deepEqual(problems(definitions(), texts()), []);
});

test("the budgets job follows every other job however it ended, unfiltered, under the released action", () => {
  const { on, jobs } = definitions()["ci.yml"];
  const { budgets, ...others } = jobs;
  assert.deepEqual([...budgets.needs].sort(), Object.keys(others).sort());
  assert.equal(budgets.if, "always()");
  assert.deepEqual(
    Object.keys(on.pull_request ?? {}),
    [],
    "no path, type or draft filter, so drafts and unrelated paths are reported",
  );
  assert.deepEqual(on.push, { branches: ["main"] });
  for (const job of Object.values(others))
    assert.equal(job.if, "github.event_name == 'pull_request'");
  assert.deepEqual(budgets.permissions, {
    contents: "read",
    actions: "read",
    "pull-requests": "write",
  });
  assert.equal(budgets.concurrency["cancel-in-progress"], false);
  assert.equal(budgets.steps[0].with["fetch-depth"], 0);
  const uses = budgets.steps[1].uses;
  const [action, tag] = uses.split("@");
  assert.equal(action, "nickderobertis/onebudgetspec-github-action");
  const declared = parse(
    readFileSync(new URL(`../reporting/action-${tag}.yml`, import.meta.url), "utf8"),
  ).inputs;
  const given = budgets.steps[1].with;
  for (const name of Object.keys(given)) assert.ok(declared[name], `the action declares ${name}`);
  for (const [name, input] of Object.entries(declared))
    if (input.required) assert.ok(given[name], `the job gives ${name}`);
  assert.deepEqual(given["enforcement-exclude-labels"].split(","), ["release-tail", "report-only"]);
  const governance = JSON.parse(
    readFileSync(new URL("../governance.json", import.meta.url), "utf8"),
  );
  assert.deepEqual(governance, { required_checks: REQUIRED, strict: false });
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
    "runs lint, test-linux, test-windows, test-macos, budgets, deny": (all) => {
      all["ci.yml"].jobs.deny = { "runs-on": "ubuntu-latest", steps: [{ run: "just deny" }] };
    },
    "a pull request opened from release-plz-2026-10-31 runs lint, test-linux, test-windows": (
      all,
    ) => {
      all["ci.yml"].jobs["test-macos"].if =
        "github.event_name == 'pull_request' && !startsWith(github.head_ref, 'release-plz-')";
    },
    "a pull request opened from release-plz-2026-10-31 (fork) runs lint, test-linux, test-windows":
      (all) => {
        all["ci.yml"].jobs["test-macos"].if =
          "github.event_name == 'pull_request' && !startsWith(github.head_ref, 'release-plz-')";
      },
    "a pull request opened from release-plz-2026-10-31 runs lint, test-linux, test-macos, budgets":
      (all) => {
        all["ci.yml"].jobs["test-windows"].if =
          "github.event_name == 'pull_request' && !startsWith(github.head_ref, 'release-plz-')";
      },
    "a pull request opened from feature runs lint, test-linux, test-macos, budgets": (all) => {
      all["ci.yml"].jobs["test-windows"].if = "github.event_name == 'push'";
    },
    "visual-docs.yml runs": (all) => {
      all["visual-docs.yml"].on.pull_request.types.push("synchronize");
    },
    "a push to main runs ci.yml's lint,budgets": (all) => {
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

test("a dedicated promotion push schedules only the existing Linux release-plz job", () => {
  const runs = running(
    graph({ event: "push", branch: "cache-promote/3877" }, { macosRequired: false }),
  );
  assert.deepEqual(Object.keys(runs), ["release-plz.yml"]);
  assert.deepEqual(
    runs["release-plz.yml"].map((row) => [row.name, row.macos]),
    [["release-plz", false]],
  );
  for (const branch of ["feature", "cache-promote-wrong/3877"])
    assert.deepEqual(running(graph({ event: "push", branch }, { macosRequired: false })), {});
});
