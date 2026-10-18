import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { test } from "node:test";
import {
  digest,
  evaluate,
  graph,
  jobs,
  matrices,
  read,
  resolve as resolveValue,
  root,
  workflows,
} from "../workflow-graph.mjs";
import { Lexer } from "../actions-expressions.mjs";
import { base, classify, event, files, heads, history, revision } from "./policy-fixture.mjs";

test("every known non-PR automatic event stays Mac-free whatever the diff", () => {
  for (const name of [
    "push",
    "release",
    "workflow_run",
    "repository_dispatch",
    "schedule",
    "workflow_dispatch",
    "merge_group",
  ])
    for (const head of [heads.source, heads.version, heads.dependency]) {
      const result = classify({ event_name: name }, head, base, name);
      assert.deepEqual(
        [result.isReleaseOnly, result.macosRequired],
        [false, false],
        `${name}: ${result.reason}`,
      );
    }
});

test("malformed product versions, non-version uv content and empty diffs never authorize omission", () => {
  assert.equal(classify(event("release-plz-123"), base, base).macosRequired, true);
  for (const changed of [
    { "pyproject.toml": files["pyproject.toml"].replace("1.2.3", "01.2.3") },
    { "uv.lock": files["uv.lock"].replace('editable = "."', 'editable = "other"') },
    {
      "sdks/python/uv.lock": files["sdks/python/uv.lock"].replace(
        'virtual = "."',
        'virtual = "other"',
      ),
    },
    { "uv.lock": "version = 1\n" },
  ])
    assert.equal(classify(event("release-plz-123"), revision(changed)).macosRequired, true);
});

test("repository identities and nested uv lock data fail conservatively", () => {
  for (const identity of ["", "malformed", {}, ["owner/repo"], "owner/", "/repo"]) {
    const payload = event("release-plz-123");
    payload.pull_request.head.repo.full_name = identity;
    payload.pull_request.base.repo.full_name = identity;
    assert.equal(classify(payload).macosRequired, true);
  }
  for (const changed of [
    {
      "sdks/python/uv.lock": files["sdks/python/uv.lock"].replace(
        'specifier = "==1.2.3"',
        'specifier = "==1.2.3", extra = "external-change"',
      ),
    },
    { "uv.lock": `${files["uv.lock"]}metadata = "invalid"\n` },
  ])
    assert.equal(classify(event("release-plz-123"), revision(changed)).macosRequired, true);
});

test("member changelog additions are releases; deleted changelogs and malformed refs are conservative", () => {
  assert.equal(
    classify(
      event("release-plz-123"),
      revision({ "crates/onetaskgraph-core/CHANGELOG.md": "# New release" }),
    ).isReleaseOnly,
    true,
  );
  assert.equal(
    classify(event("release-plz-123"), revision({ "CHANGELOG.md": null })).macosRequired,
    true,
  );
  const malformed = event("release-plz-123");
  malformed.pull_request.head.ref = {};
  assert.equal(classify(malformed).macosRequired, true);
});

test("real classification composes with actual workflow routing for forged, mixed, fork and unreadable PRs", () => {
  for (const [payload, head] of [
    [event("feature"), heads.source],
    [event("release-plz-123"), heads.version],
    [event("release-plz-forged"), heads.source],
    [event("release-plz-123"), heads.dependency],
    [event("release-plz-123", true), heads.version],
    ["{malformed", heads.version],
  ]) {
    const decision = classify(payload, head);
    // An unreadable payload is a pull request on no branch GitHub could name.
    const pr = typeof payload === "string" ? { head: { ref: "", repo: {} } } : payload.pull_request;
    const scenario = {
      event: "pull_request",
      action: "synchronize",
      headRef: pr.head.ref,
      fork: pr.head.repo.fork,
    };
    const ci = graph(scenario, decision).find((run) => run.workflow === "ci.yml");
    // test-macos is skipped on this repository's release-plz branch alone, and never on a
    // pull request the classifier has not authorized to omit macOS without it saying so:
    // a forged release branch carrying source runs no Mac job, and its carrier goes red.
    const releaseBranch = !pr.head.repo.fork && pr.head.ref.startsWith("release-plz-");
    const tested = ci.rows.some((row) => row.job === "test-macos");
    assert.equal(tested, !releaseBranch);
    assert.equal(
      ci.rows.some((row) => row.job === "macos"),
      false,
    );
    if (tested) assert.ok(decision.macosRequired);
    for (const name of ["check (macos-latest)", "install path (macos-latest)"])
      assert.ok(ci.rows.some((row) => row.name === name));
    if (!tested && decision.macosRequired) {
      const needs = {
        "macos-lanes": { result: "success", outputs: { macosRequired: "true", reason: "x" } },
        macos: { result: "skipped" },
        "test-macos": { result: "skipped" },
      };
      writeFileSync(resolve(history, "needs.json"), JSON.stringify(needs));
      assert.notEqual(
        spawnSync(process.execPath, [resolve(history, ".github/macos-result.mjs"), "needs.json"], {
          cwd: history,
          encoding: "utf8",
        }).status,
        0,
      );
    }
  }
});

test("production classifier step uses its declared event and exact base/head environment", () => {
  const step = workflows()["ci.yml"].jobs["macos-lanes"].steps.find((step) => step.id === "select");
  for (const [payload, head] of [
    [event("feature"), heads.source],
    [event("release-plz-123"), heads.version],
    [event("release-plz-123"), heads.dependency],
    [event("release-plz-123", true), heads.version],
  ]) {
    payload.pull_request.base.sha = base;
    payload.pull_request.head.sha = head;
    const input = resolve(history, "event.json");
    writeFileSync(input, JSON.stringify(payload));
    const output = resolve(history, "github-output");
    rmSync(output, { force: true });
    const env = resolveValue(step.env, { github: { event: payload } });
    const result = spawnSync("bash", ["-c", step.run], {
      cwd: history,
      encoding: "utf8",
      env: {
        ...process.env,
        ...env,
        GITHUB_EVENT_NAME: "pull_request",
        GITHUB_EVENT_PATH: input,
        GITHUB_OUTPUT: output,
      },
    });
    assert.equal(result.status, 0, result.stderr);
    const actual = readFileSync(output, "utf8");
    const expected = classify(payload, head);
    assert.equal(actual, `macosRequired=${expected.macosRequired}\nreason=${expected.reason}\n`);
  }
});

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
    definitions["release.yml"].jobs["publish-npm"].if = expression;
    assert.throws(() => validatePublishControls(definitions), /Unmeasured|Unresolved/);
  }
});

const fixtureClasses = JSON.parse(read(".github/event-classes.json"));
const requiredNames = [
  "check (macos-latest)",
  "check (ubuntu-latest)",
  "check (windows-latest)",
  "deny",
  "install path (macos-latest)",
  "install path (ubuntu-latest)",
  "install path (windows-latest)",
  "llmlint",
  "pr-title",
  "visual-docs / report (x86_64, shots/current, shots/verify, Visual docs)",
];

test("workflow matrices, event fan-out and exact release head model record scheduling telemetry", () => {
  validatePublishControls();
  const scenarios = [];
  const lifecycles = [];
  const classifications = new Map();
  for (const diff of ["source", "version", "dependency"])
    for (const fork of [false, true])
      for (const entry of fixtureClasses)
        for (let optional = 0; optional < 2 ** publishControls.length; optional++) {
          const payload =
            entry.event === "pull_request"
              ? event(diff === "version" ? "release-plz-123" : "feature", fork)
              : { event_name: entry.event };
          const key = JSON.stringify([entry.event, diff, fork]);
          if (!classifications.has(key)) classifications.set(key, classify(payload, heads[diff]));
          const policyResult = classifications.get(key);
          const scenario = {
            ...entry,
            diff,
            fork,
            headRef: diff === "version" ? "release-plz-123" : "feature",
            vars: publicationVariables(optional),
            // A push to main is decided over real revisions: rust-cache-warm.yml runs its own
            // release-merge step against this history.
            ...(entry.event === "push"
              ? { revisions: { cwd: history, before: base, after: heads[diff] } }
              : {}),
          };
          const runs = graph(scenario, policyResult);
          for (const run of runs) {
            const macs = run.rows.filter((r) => r.macos);
            if (entry.event === "pull_request") {
              const names = run.rows.map((r) => r.name);
              if (run.workflow === "ci.yml")
                for (const name of requiredNames.slice(0, -1))
                  assert.equal(names.filter((n) => n === name).length, 1, name);
              if (run.workflow === "visual-docs.yml")
                assert.ok(
                  names.includes(
                    "visual-docs / report (x86_64, shots/current, shots/verify, Visual docs)",
                  ),
                );
            }
            for (const row of macs)
              assert.ok(
                (entry.event === "pull_request" &&
                  policyResult.macosRequired &&
                  run.workflow === "ci.yml") ||
                  (entry.event === "release" && run.workflow === "release.yml") ||
                  (["manual-opt-in", "manual-distribution"].includes(entry.id) &&
                    run.workflow === "ci.yml") ||
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
            if (run.workflow === "ci.yml" && entry.event === "workflow_dispatch") {
              assert.equal(
                macs.some((row) => row.job === "macos"),
                entry.id === "manual-opt-in",
              );
              assert.equal(
                macs.some((row) => row.job === "distribution-diagnostic"),
                entry.id === "manual-distribution",
              );
            }
            if (entry.event === "release" && run.workflow === "release.yml")
              assert.ok(macs.some((r) => r.job === "macos-assets-carriers-and-wheels"));
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

test("malformed lock records and Bun inventory disagreement require coverage through the classifier", () => {
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
    const result = classify(event("release-plz-123"), revision(changes));
    assert.equal(result.isReleaseOnly, false);
    assert.equal(result.macosRequired, true);
    assert.equal(result.reason, "Unreadable release diff requires coverage");
  }
});
