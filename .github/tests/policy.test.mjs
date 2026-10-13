import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
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

const policy =
  process.env.MACOS_POLICY_TEST_PROGRAM ||
  fileURLToPath(new URL("../macos-policy.mjs", import.meta.url));
const scratchRoot = process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir();
const history = mkdtempSync(resolve(scratchRoot, "macos-policy-"));
function git(...args) {
  return execFileSync("git", args, { cwd: history, encoding: "utf8" }).trim();
}
function put(path, text) {
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

const files = {
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
      ["linux-x64", "linux-arm64", "darwin-x64", "darwin-arm64", "win32-x64"].map((x) => [
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
    'version = 1\n[[package]]\nname = "onetaskgraph-sdk"\nversion = "1.2.3"\nsource = { virtual = "." }\ndependencies = [{name = "onetaskgraph-cli", version = "1.2.3"}]\n[package.metadata]\nrequires-dist = [{name = "onetaskgraph-cli", specifier = "==1.2.3"}]\n',
  "crates/onetaskgraph-core/fixtures/Cargo.toml":
    '[package]\nname = "fixture"\nversion = "1.2.3"\n',
  "source.rs": "fn main() {}\n",
  "CHANGELOG.md": "# Changes\n",
};
for (const [p, s] of Object.entries(files)) put(p, s);
git("add", ".");
git("commit", "-qm", "base");
const base = git("rev-parse", "HEAD");
function revision(changes) {
  git("checkout", "-q", base);
  for (const [p, s] of Object.entries(changes)) {
    if (s === null) rmSync(resolve(history, p));
    else put(p, s);
  }
  git("add", ".");
  git("commit", "-qm", "scenario");
  return git("rev-parse", "HEAD");
}
const versions = Object.fromEntries(
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
const heads = {
  version: revision(versions),
  source: revision({ "source.rs": 'fn main() { panic!("changed"); }\n' }),
  dependency: revision({
    "pyproject.toml": files["pyproject.toml"].replace("external==1.0.0", "external==2.0.0"),
  }),
};
function event(ref = "feature", fork = false, author = "nickderobertis") {
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
function classify(payload, head = heads.version, baseRef = base, authoritativeEvent = "") {
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
      env: { ...process.env, GITHUB_EVENT_NAME: authoritativeEvent, GITHUB_OUTPUT: output },
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

test("classifier runs on real histories: version-only authority is branch, ownership and semantics", () => {
  assert.deepEqual(
    Object.entries(classify(event("release-plz-123"))).filter(([key]) => key !== "reason"),
    [
      ["isReleaseOnly", true],
      ["macosRequired", false],
    ],
  );
  for (const payload of [
    event("feature", false, "github-actions[bot]"),
    event("feature"),
    event("release-plz-123", true),
    event("release-plz-forged", false),
  ]) {
    const head =
      payload.pull_request.head.ref === "release-plz-forged" ? heads.source : heads.version;
    assert.equal(classify(payload, head).macosRequired, true);
  }
  for (const head of [heads.source, heads.dependency])
    assert.equal(classify(event("release-plz-123"), head).macosRequired, true);
  for (const input of ["{broken", {}, null, { pull_request: { head: { ref: "release-plz-1" } } }])
    assert.equal(classify(input).macosRequired, true);
  assert.equal(
    classify(event("release-plz-123"), heads.version, "missing-ref").macosRequired,
    true,
  );
});

test("semantic comparison refuses mixed fields, dependency lock versions and unrelated changelogs", () => {
  const changes = [
    {
      "Cargo.lock": files["Cargo.lock"].replace(
        'name = "external"\nversion = "1.2.3"',
        'name = "external"\nversion = "1.2.4"',
      ),
    },
    { "Cargo.lock": files["Cargo.lock"].replace('checksum = "abc"', 'checksum = "def"') },
    { "bun.lock": files["bun.lock"].replace("external@1.0.0", "external@2.0.0") },
    {
      "sdks/typescript/src/index.ts": files["sdks/typescript/src/index.ts"].replace(
        "OTHER = 1",
        "OTHER = 2",
      ),
    },
    {
      "npm/cli/package.json": files["npm/cli/package.json"].replace(
        '@onetaskgraph/cli"',
        '@forged/cli"',
      ),
    },
    { "fixtures/CHANGELOG.md": "# unrelated" },
    { "crates/onetaskgraph-core/fixtures/CHANGELOG.md": "# fixture changes" },
    { "fixtures/Cargo.toml": '[package]\nversion = "1.2.4"' },
    { "pyproject.toml": "malformed" },
  ];
  for (const changed of changes)
    assert.equal(
      classify(event("release-plz-123"), revision(changed)).macosRequired,
      true,
      JSON.stringify(changed),
    );
});

test("Actions event environment overrides payload kinds and unreadable kinds require coverage", () => {
  const conflicting = { ...event("feature"), event_name: "push" };
  assert.equal(classify(conflicting, heads.source, base, "pull_request").macosRequired, true);
  assert.equal(
    classify(event("release-plz-123"), heads.version, base, "push").macosRequired,
    false,
  );
  assert.equal(
    classify({ event_name: "unknown" }, heads.source, base, "merge_group").macosRequired,
    false,
  );
  for (const event_name of ["unknown", {}, 42])
    assert.equal(classify({ event_name }).macosRequired, true);
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
    const ci = graph({ event: "pull_request", action: "synchronize" }, decision).find(
      (run) => run.workflow === "ci.yml",
    );
    assert.equal(
      ci.rows.some((row) => row.job === "macos"),
      decision.macosRequired,
    );
    for (const name of ["check (macos-latest)", "install path (macos-latest)"])
      assert.ok(ci.rows.some((row) => row.name === name));
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

test("actual carrier run bodies preserve success, failure, cancellation, missing and authorized omission", () => {
  const ci = workflows()["ci.yml"];
  for (const id of ["macos-check", "macos-install"]) {
    const job = ci.jobs[id];
    assert.equal(job.if, `\${{ always() }}`);
    const run = job.steps.find((s) => s.run)?.run;
    assert.match(run, /macos-result\.mjs/);
    for (const diagnostic of [null, false, "invalid", {}]) {
      const needs = {
        "macos-lanes": { result: "success", outputs: { macosRequired: "false", diagnostic } },
        macos: { result: "skipped" },
      };
      assert.notEqual(
        spawnSync("bash", ["-c", run], {
          cwd: history,
          encoding: "utf8",
          env: { ...process.env, NEEDS_JSON: JSON.stringify(needs) },
        }).status,
        0,
      );
    }
    for (const diagnostic of ["true", "false"])
      for (const result of ["success", "skipped", "cancelled", "failure"]) {
        const needs = {
          "macos-lanes": {
            result: "success",
            outputs: { macosRequired: "false", diagnostic, reason: "manual" },
          },
          macos: { result },
        };
        const outcome = spawnSync("bash", ["-c", run], {
          cwd: history,
          encoding: "utf8",
          env: { ...process.env, NEEDS_JSON: JSON.stringify(needs) },
        });
        assert.equal(
          outcome.status === 0,
          diagnostic === "true" ? result === "success" : result === "skipped",
        );
      }
    for (const classification of ["success", "failure", "cancelled", "skipped", undefined])
      for (const required of ["true", "false", undefined])
        for (const result of ["success", "failure", "cancelled", "skipped", undefined]) {
          const needs = {
            "macos-lanes": {
              result: classification,
              outputs: { macosRequired: required, reason: "semantic release omission" },
            },
            macos: { result },
          };
          const outcome = spawnSync("bash", ["-c", run], {
            cwd: history,
            encoding: "utf8",
            env: { ...process.env, NEEDS_JSON: JSON.stringify(needs) },
          });
          const allowed =
            classification === "success" &&
            ((required === "true" && result === "success") ||
              (required === "false" && result === "skipped"));
          assert.equal(outcome.status === 0, allowed, JSON.stringify(needs));
          if (allowed && required === "false") {
            assert.match(outcome.stdout, /omitted/);
            assert.doesNotMatch(outcome.stdout, /proofs passed/);
          }
        }
  }
  rmSync(resolve(history, "needs.json"), { force: true });
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
  const scenarios = [];
  const lifecycles = [];
  const classifications = new Map();
  for (const diff of ["source", "version", "dependency"])
    for (const fork of [false, true])
      for (const entry of fixtureClasses)
        for (let optional = 0; optional < 8; optional++) {
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
            vars: {
              CARGO_PUBLISH: optional & 1 ? "true" : "false",
              PYPI_PUBLISH: optional & 2 ? "true" : "false",
              NPM_PUBLISH: optional & 4 ? "true" : "false",
            },
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
                  (entry.id === "manual-opt-in" && run.workflow === "ci.yml"),
                JSON.stringify(row),
              );
            if (entry.event === "pull_request" && run.workflow === "ci.yml")
              assert.equal(
                macs.some((r) => r.job === "macos"),
                policyResult.macosRequired,
              );
            if (run.workflow === "ci.yml" && ["manual-default", "manual-opt-in"].includes(entry.id))
              assert.equal(
                macs.some((row) => row.job === "macos"),
                entry.id === "manual-opt-in",
              );
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
      for (let optional = 0; optional < 8; optional++) {
        const sequence = [
          ["pr-synchronize", diff, fork],
          ["base-push", diff, false],
          ["pr-opened", "version", false],
          ["release-pr-merge", "version", false],
          ["release-tag", "version", false],
          ["release-published", "version", false],
          ["downstream", "version", false],
          ["dispatch", "version", false],
          ["schedule", "version", false],
        ];
        const runs = sequence.flatMap(([id, d, f]) =>
          scenarios
            .find(
              (s) =>
                s.id === id &&
                s.diff === d &&
                s.fork === f &&
                s.vars.CARGO_PUBLISH === (optional & 1 ? "true" : "false") &&
                s.vars.PYPI_PUBLISH === (optional & 2 ? "true" : "false") &&
                s.vars.NPM_PUBLISH === (optional & 4 ? "true" : "false"),
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
              vars: {
                CARGO_PUBLISH: optional & 1 ? "true" : "false",
                PYPI_PUBLISH: optional & 2 ? "true" : "false",
                NPM_PUBLISH: optional & 4 ? "true" : "false",
              },
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
