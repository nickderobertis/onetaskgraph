import "../../test-setup.mjs";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { test } from "node:test";
import { Lexer } from "../../actions-expressions.mjs";
import {
  digest,
  evaluate,
  graph,
  jobs,
  matrices,
  read,
  root,
  workflows,
} from "../../workflow-graph.mjs";
import { base, files, heads, history, revision, scratchRoot, semantic } from "./policy-fixture.mjs";

const publishControls = ["CARGO_PUBLISH", "PYPI_PUBLISH", "NPM_PUBLISH"];
function validatePublishControls(definitions = workflows()) {
  const found = new Set();
  function visit(value) {
    if (!value || typeof value !== "object") return;
    for (const [key, entry] of Object.entries(value)) {
      if (key === "if" && typeof entry === "string") {
        const expression = entry.trim().replace(/^\$\{\{\s*|\s*\}\}$/g, "");
        const { tokens } = new Lexer(expression).lex();
        for (let i = 0; i < tokens.length; i++) {
          if (tokens[i].lexeme !== "vars") continue;
          const next = tokens[i + 1];
          const field = tokens[i + 2];
          if (next?.lexeme === "." && /^[A-Za-z_][A-Za-z0-9_]*$/.test(field?.lexeme || ""))
            found.add(field.lexeme);
          else if (
            next?.lexeme === "[" &&
            typeof field?.value === "string" &&
            tokens[i + 3]?.lexeme === "]"
          )
            found.add(field.value);
          else throw Error("Unresolved publication variable in workflow condition");
        }
      } else visit(entry);
    }
  }
  visit(definitions);
  assert.deepEqual(
    [...found].sort(),
    [...publishControls].sort(),
    "Unmeasured workflow publication controls",
  );
}
function publicationVariables(optional) {
  return Object.fromEntries(
    publishControls.map((name, index) => [name, optional & (1 << index) ? "true" : "false"]),
  );
}

test("publication workload reconciles actual workflow condition variables", () => {
  validatePublishControls();
  for (const expression of [
    "vars.EXTRA_PUBLISH == 'true'",
    "vars['EXTRA_PUBLISH'] == 'true'",
    "vars[inputs.control] == 'true'",
  ]) {
    const definitions = workflows();
    definitions["release.yml"].jobs.publish.if = expression;
    assert.throws(() => validatePublishControls(definitions), /Unmeasured|Unresolved/);
  }
});

const fixtureClasses = JSON.parse(read(".github/event-classes.json"));
const requiredNames = ["lint", "test-linux", "test-windows", "test-macos", "budgets"];

test("workflow matrices, event fan-out and exact release head model record scheduling telemetry", () => {
  validatePublishControls();
  const scenarios = [];
  const lifecycles = [];
  for (const diff of ["source", "version", "dependency"])
    for (const fork of [false, true])
      for (const entry of fixtureClasses)
        for (let optional = 0; optional < 2 ** publishControls.length; optional++) {
          // The diff is known from this fixture's real history. Every PR allocates macOS,
          // independently of release semantics, ownership, author and branch spelling.
          const policyResult = {
            isReleaseOnly: diff === "version",
            macosRequired: entry.event === "pull_request",
            reason: "Every pull request runs all four required jobs",
          };
          const scenario = {
            ...entry,
            diff,
            fork,
            headRef: diff === "version" ? "release-plz-123" : "feature",
            vars: publicationVariables(optional),
            // A push to main is decided over real revisions: rust-cache-warm.yml runs its own
            // release-merge step against this history.
            ...(entry.event === "push"
              ? {
                  revisions: { cwd: history, before: base, after: heads[diff] },
                  // release-plz's squash is what a version-only push to main is.
                  headCommitMessage:
                    diff === "version" ? "chore: release v1.2.4 (#2)" : "fix: x (#1)",
                }
              : {}),
          };
          const runs = graph(scenario, policyResult);
          for (const run of runs) {
            const macs = run.rows.filter((r) => r.macos);
            if (entry.event === "pull_request") {
              const names = run.rows.map((r) => r.name);
              if (run.workflow === "ci.yml") assert.deepEqual(names, requiredNames);
              else assert.deepEqual(names, [], `${run.workflow} runs on a pull request`);
            }
            for (const row of macs)
              assert.ok(
                (entry.event === "pull_request" &&
                  policyResult.macosRequired &&
                  run.workflow === "ci.yml") ||
                  (entry.event === "release" && run.workflow === "release.yml") ||
                  // The one Mac job a push to main schedules, by the user's decision: the
                  // build-only Rust cache warmer test-macos restores (AGENTS.md).
                  (entry.event === "push" &&
                    entry.branch === "main" &&
                    run.workflow === "rust-cache-warm.yml" &&
                    row.job === "warm-macos"),
                JSON.stringify(row),
              );
            // And it warms on every push but a release merge: one changing release versions
            // and changelogs alone, which is what the release pull request's merge is.
            if (
              entry.event === "push" &&
              entry.branch === "main" &&
              run.workflow === "rust-cache-warm.yml"
            )
              assert.deepEqual(
                macs.map((row) => row.job),
                diff === "version" ? [] : ["warm-macos"],
                `${entry.id} (${diff})`,
              );
            if (entry.event === "pull_request" && run.workflow === "ci.yml")
              assert.equal(
                macs.some((r) => r.job === "test-macos"),
                policyResult.macosRequired,
              );
            if (entry.event === "release" && run.workflow === "release.yml")
              assert.ok(macs.some((r) => r.job === "build-macos"));
            run.macJobs = macs.length;
          }
          scenarios.push({ ...scenario, policy: policyResult, runs });
        }
  // One pushed change fans into its PR run, main merge, release PR, release PR merge,
  // release-tag push, published release, and downstream automatic events. The workflow
  // event declarations decide which runs exist at each phase.
  for (const diff of ["source", "version", "dependency"])
    for (const fork of [false, true])
      for (let optional = 0; optional < 2 ** publishControls.length; optional++) {
        const sequence = [["pr-synchronize", diff, fork]];
        for (const entry of JSON.parse(read(".github/event-classes.json"))) {
          if (["pull_request", "workflow_dispatch"].includes(entry.event)) continue;
          sequence.push([entry.id, entry.id === "base-push" ? diff : "version", false]);
          if (entry.id === "base-push") sequence.push(["pr-opened", "version", false]);
        }
        const runs = sequence.flatMap(([id, d, f]) =>
          scenarios
            .find(
              (s) =>
                s.id === id &&
                s.diff === d &&
                s.fork === f &&
                publishControls.every(
                  (name) => s.vars[name] === publicationVariables(optional)[name],
                ),
            )
            .runs.map((r) => ({ ...r, phase: id })),
        );
        // Each merge also closes its PR; actual closed-event declarations determine
        // the downstream run inventory without adding another user-push scenario.
        for (const phase of ["base-push", "release-pr-merge"]) {
          for (const run of graph(
            {
              event: "pull_request",
              action: "closed",
              fork: phase === "base-push" && fork,
              vars: publicationVariables(optional),
            },
            { macosRequired: false, reason: "closed PR has no CI trigger" },
          ))
            runs.push({
              ...run,
              phase: `${phase}-closed`,
              macJobs: run.rows.filter((row) => row.macos).length,
            });
        }
        lifecycles.push({
          diff,
          fork,
          optional,
          runs,
          macRuns: runs.filter((r) => r.macJobs > 0).length,
        });
      }
  mkdirSync(new URL(".github/.telemetry/", root), { recursive: true });
  writeFileSync(
    new URL(".github/.telemetry/macos-policy.json", root),
    JSON.stringify({ schemaVersion: 1, sourceDigest: digest(), scenarios, lifecycles }, null, 2) +
      "\n",
  );
});

test("unresolved expressions, runners, calls and needs fail explicitly; actual matrix expansion honors include/exclude", () => {
  assert.deepEqual(
    matrices(
      {
        os: ["ubuntu-latest", "windows-latest"],
        arch: ["x64"],
        exclude: [{ os: "windows-latest" }],
        include: [{ os: "macos-latest", arch: "arm64" }],
      },
      {},
    ),
    [
      { os: "ubuntu-latest", arch: "x64" },
      { os: "macos-latest", arch: "arm64" },
    ],
  );
  for (const job of [
    { "runs-on": `\${{ mystery.os }}` },
    { uses: "unknown/repo/.github/workflows/x.yml@v1" },
    { "runs-on": [] },
    { "runs-on": `\${{ inputs.runner }}` },
  ])
    assert.throws(
      () => jobs({ jobs: { bad: job } }, { event: "push" }, { macosRequired: false }),
      /Unresolved|context|Unknown|Unrecognized/i,
    );
  assert.throws(
    () =>
      jobs(
        { jobs: { bad: { needs: "missing", "runs-on": "macos-latest" } } },
        { event: "push" },
        {},
      ),
    /Unresolved job needs/,
  );
  assert.deepEqual(
    matrices({ os: ["ubuntu-latest"], include: [{ label: "first" }, { label: "second" }] }, {}),
    [{ os: "ubuntu-latest", label: "second" }],
  );
  assert.equal(evaluate("always()", { needs: { proof: { result: "failure" } } }), true);
});

test.after(() => rmSync(history, { recursive: true, force: true }));

// A push the release-merge step cannot classify still warms macOS: only a decided `true` skips it.
const warmMacos = (revisions) =>
  graph(
    { id: "push-main", event: "push", branch: "main", revisions },
    {
      macosRequired: false,
      reason: "a push",
    },
  )
    .find((run) => run.workflow === "rust-cache-warm.yml")
    .rows.filter((row) => row.macos)
    .map((row) => row.job);

test("a push the release-merge step cannot read still schedules warm-macos", () => {
  // GitHub sends forty zeros as `before` for a branch's first push; neither revision resolves.
  for (const before of ["0".repeat(40), "deadbeef".repeat(5)])
    assert.deepEqual(warmMacos({ cwd: history, before, after: heads.version }), ["warm-macos"]);
  // The same release merge, readable, is the one push that warms no Mac.
  assert.deepEqual(warmMacos({ cwd: history, before: base, after: heads.version }), []);
});

test("a classifier that fails or answers anything but true makes the decide step answer false", () => {
  const decide = workflows()["rust-cache-warm.yml"].jobs["release-merge"].steps.find(
    (step) => step.id === "decide",
  ).run;
  const scratch = mkdtempSync(resolve(scratchRoot, "release-merge-shim-"));
  try {
    const shims = resolve(scratch, "bin");
    mkdirSync(shims);
    for (const [body, label] of [
      ["printf '{\"version-only\": tru'", "malformed JSON"],
      ["printf 'maybe\\n'", "another word"],
      ["printf ''", "nothing"],
      ["printf 'true\\n'; exit 3", "true, then a failure"],
      ["exit 1", "a failure alone"],
    ]) {
      writeFileSync(resolve(shims, "python3"), `#!/usr/bin/env bash\n${body}\n`);
      chmodSync(resolve(shims, "python3"), 0o755);
      const output = resolve(scratch, "output");
      rmSync(output, { force: true });
      // As GitHub runs a `shell: bash` step.
      const ran = spawnSync("bash", ["--noprofile", "--norc", "-eo", "pipefail", "-c", decide], {
        cwd: root,
        env: {
          ...process.env,
          PATH: `${shims}${process.platform === "win32" ? ";" : ":"}${process.env.PATH}`,
          BEFORE: base,
          AFTER: heads.version,
          GITHUB_OUTPUT: output,
        },
        encoding: "utf8",
      });
      assert.equal(ran.status, 0, `${label}: ${ran.stderr}`);
      assert.equal(readFileSync(output, "utf8").trim(), "version-only=false", label);
    }
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});

test("a release-merge job that fails or reports no answer leaves warm-macos scheduled", () => {
  const condition = workflows()["rust-cache-warm.yml"].jobs["warm-macos"].if;
  // GitHub's rule, as jobs() in workflow-graph.mjs applies it: a condition naming no status
  // function also needs every dependency to have succeeded.
  const runs = (result, outputs) =>
    (result === "success" || /\b(always|cancelled|failure|success)\(/.test(condition)) &&
    Boolean(evaluate(condition, { needs: { "release-merge": { result, outputs } } }));
  assert.equal(runs("failure", {}), true);
  assert.equal(runs("success", {}), true);
  assert.equal(runs("success", { "version-only": "false" }), true);
  assert.equal(runs("success", { "version-only": "true" }), false);
  assert.equal(runs("cancelled", {}), false);
});

test("malformed lock records and Bun inventory disagreement require coverage through the semantic CLI", () => {
  const malformed = [];
  for (const path of ["Cargo.lock", "uv.lock"]) {
    for (const packages of ["package = []", 'package = "invalid"', "package = [1]"])
      malformed.push({ [path]: `version = 1\n${packages}\n` });
    const prefix = files[path];
    for (const record of [
      'dependencies = "invalid"',
      'metadata = "invalid"',
      'metadata = { requires-dist = "invalid" }',
      "metadata = { requires-dist = [1] }",
      path === "Cargo.lock" ? "dependencies = [1]" : 'dependencies = ["invalid"]',
    ])
      malformed.push({ [path]: `${prefix}${record}\n` });
  }
  // Valid Bun syntax, but the updater's registered line patterns cannot mask
  // compact fields; this must reach the inventory disagreement refusal.
  malformed.push({ "bun.lock": files["bun.lock"].replace(/\n\s*/g, "") });
  for (const changes of malformed) {
    const result = semantic(revision(changes));
    assert.equal(result.isReleaseOnly, false);
    assert.notEqual(result.status, 0);
  }
});
