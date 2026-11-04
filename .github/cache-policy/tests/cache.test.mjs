import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { createServer } from "node:net";
import { tmpdir } from "node:os";
import { dirname, isAbsolute, join, resolve, sep } from "node:path";
import { after, test } from "node:test";
import { fileURLToPath } from "node:url";
import { parse } from "yaml";
import { OP_NX_COMMIT, PROMOTION_BRANCH_PATTERN } from "../../../scripts/nx-remote-cache.mjs";
import { evaluate, workflows } from "../../workflow-graph.mjs";

const root = fileURLToPath(new URL("../../../", import.meta.url));
const scratch = mkdtempSync(
  join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir(), "nx-cache-"),
);
after(() => rmSync(scratch, { recursive: true, force: true }));
let cases = 0;
const tree = () => {
  const dir = join(scratch, String(cases++));
  mkdirSync(dir);
  return dir;
};
function command(program, args, cwd, env = {}) {
  const result = spawnSync(program, args, {
    cwd,
    env: {
      ...process.env,
      NODE_TEST_CONTEXT: undefined,
      GIT_DIR: undefined,
      GIT_WORK_TREE: undefined,
      GIT_INDEX_FILE: undefined,
      GIT_COMMON_DIR: undefined,
      ...env,
    },
    encoding: "utf8",
    timeout: 120_000,
  });
  assert.equal(result.status, 0, result.stdout + result.stderr);
  return result.stdout.trim();
}

const object = (value) => value !== null && typeof value === "object" && !Array.isArray(value);
const text = (value) => typeof value === "string" && value.length > 0;
const sha = (value) => typeof value === "string" && /^[a-f0-9]{40}$/.test(value);
const integer = (value) => Number.isSafeInteger(value) && value >= 0;
const loopback = (value) => {
  if (typeof value !== "string") return false;
  const match = /^http:\/\/127\.0\.0\.1:(\d{1,5})\/?$/.exec(value);
  return match !== null && Number(match[1]) > 0 && Number(match[1]) <= 65535;
};
function jsonFile(file, shape, description) {
  const value = JSON.parse(readFileSync(file, "utf8"));
  assert.ok(shape(value), `${file}: expected ${description}`);
  return value;
}
const executionFile = (file) =>
  jsonFile(
    file,
    (value) =>
      object(value) &&
      sha(value.head_sha) &&
      integer(value.run_id) &&
      value.run_id > 0 &&
      integer(value.attempt) &&
      value.attempt > 0 &&
      object(value.selection) &&
      value.selection.mode === "affected" &&
      sha(value.selection.base_sha) &&
      sha(value.selection.head_sha) &&
      Array.isArray(value.tasks) &&
      value.tasks.length > 0 &&
      value.tasks.every(
        (task) =>
          object(task) &&
          text(task.task) &&
          text(task.hash) &&
          ["executed", "remote_hit", "local_hit", "failed"].includes(task.result) &&
          typeof task.started_at === "string" &&
          typeof task.finished_at === "string" &&
          Number.isFinite(Date.parse(task.started_at)) &&
          Number.isFinite(Date.parse(task.finished_at)),
      ) &&
      Array.isArray(value.store_attempts) &&
      value.store_attempts.every(
        (attempt) =>
          object(attempt) &&
          ["stored", "failed", "throttled"].includes(attempt.status) &&
          typeof attempt.attempted_at === "string" &&
          Number.isFinite(Date.parse(attempt.attempted_at)),
      ),
    "task executions with immutable selection and store attempts",
  );
const manifestFile = (file) =>
  jsonFile(
    file,
    (value) =>
      object(value) &&
      integer(value.stored_bytes) &&
      Array.isArray(value.entries) &&
      value.entries.every(
        (entry) =>
          object(entry) &&
          text(entry.hash) &&
          integer(entry.bytes) &&
          ["stored", "replayed"].includes(entry.source),
      ),
    "cache manifest entries and stored bytes",
  );
const eventFile = (file) =>
  jsonFile(
    file,
    (value) => {
      const pr = value?.event?.pull_request;
      return (
        object(value) &&
        typeof value.repository === "string" &&
        /^[\w.-]+\/[\w.-]+$/.test(value.repository) &&
        object(pr) &&
        integer(pr.number) &&
        pr.number > 0 &&
        sha(pr.head?.sha) &&
        sha(pr.base?.sha) &&
        text(pr.head?.repo?.full_name) &&
        text(pr.base?.repo?.full_name) &&
        text(pr.user?.login)
      );
    },
    "recorded PR identity, author and commit ids",
  );

// Biome excludes exactly these upstream-owned copies; this drift test holds them byte-identical.
// Full-file digests, including headers: remote cache and figures from dero-skills
// 63e071b72420bd6737226a50cb8a7a7760cc73a4; version-free inputs v3 from
// fe20052d06406bb07869c5236147b6693489d52f (owned Cargo and Python pins).
const copies = {
  "scripts/actions-budget-figures.sh":
    "a3606ff3d00be139d52a7cbb2905f45e2ade99383417be31cc74cc7ce6c39eba",
  "scripts/nx-remote-cache.mjs": "4445c0d8d5f6045bb2b14b19cd03f9600b6b98f88dc689421d98d3ef996664b6",
  "scripts/nx-version-free-inputs.mjs":
    "624fed99b45a57532ef672ccb4638e4b8052dd12fdee0274fa2f23e9f2f16b60",
};
function drift(file, bytes) {
  return createHash("sha256").update(bytes).digest("hex") !== copies[file];
}
test("shared scripts remain byte-identical to the agreed upstream commit", () => {
  for (const file of Object.keys(copies)) {
    const bytes = readFileSync(join(root, file));
    assert.equal(drift(file, bytes), false, file);
    assert.equal(drift(file, Buffer.concat([bytes, Buffer.from("\n")])), true, file);
  }
  const config = jsonFile(
    join(root, "biome.json"),
    (value) =>
      object(value) &&
      object(value.files) &&
      Array.isArray(value.files.includes) &&
      value.files.includes.every((item) => typeof item === "string"),
    "Biome file selection",
  );
  for (const file of Object.keys(copies).filter((file) => file.endsWith(".mjs")))
    assert.ok(config.files.includes.includes(`!${file}`));
});

const workflow = (name) => workflows()[`${name}.yml`];
function cacheProblems(ci, release) {
  const problems = [];
  const jobs = ["lint", "test-linux", "test-windows", "test-macos"];
  if (
    ci.env?.NX_BASE !== `\${{ github.event.pull_request.base.sha }}` ||
    ci.env?.NX_HEAD !== `\${{ github.event.pull_request.head.sha }}`
  )
    problems.push("all cache reports must use the event's immutable selection");
  if (JSON.stringify(Object.keys(ci.jobs).sort()) !== JSON.stringify([...jobs].sort()))
    problems.push("CI must keep exactly the four existing jobs");
  for (const name of jobs) {
    const steps = ci.jobs[name]?.steps ?? [];
    if (
      [ci.jobs[name].env, ...steps.map((step) => step.env)].some(
        (env) => env?.NX_BASE !== undefined || env?.NX_HEAD !== undefined,
      )
    )
      problems.push(`${name}: event comparison must not be overridden`);

    if ((ci.jobs[name].permissions ?? ci.permissions)?.actions !== "read")
      problems.push(`${name}: cache workflow requires actions read`);
    const trust = steps.findIndex((step) => step.id === "nx-cache-trust");
    const sidecar = steps.findIndex((step) => step.id === "nx-cache");
    const ready = steps.findIndex((step) => step.run?.endsWith("nx-remote-cache.mjs ready"));
    const report = steps.findIndex((step) => step.run?.includes("nx-remote-cache.mjs report"));
    const run = steps.findIndex((step) => step.run?.includes(`just ci-${name}`));
    const upload = steps.findIndex(
      (step) => step.with?.name === `nx-cache-entries-\${{ github.job }}`,
    );
    if (
      !(
        trust >= 0 &&
        trust < sidecar &&
        sidecar < ready &&
        ready < run &&
        run < report &&
        report < upload
      )
    )
      problems.push(`${name}: cache steps must surround its existing Nx run`);
    if (
      steps[sidecar]?.uses !== `op-nx/github-cache/start-cache-server@${OP_NX_COMMIT}` ||
      steps[sidecar]?.background !== true
    )
      problems.push(`${name}: sidecar must be pinned and backgrounded`);
    if (
      steps[sidecar]?.env?.CACHE_READ_ONLY !==
      `\${{ steps.nx-cache-trust.outputs.read_only == 'true' && '1' || '' }}`
    )
      problems.push(`${name}: sidecar must use trust's read-only answer`);
    if (
      steps[report]?.if !== "always()" ||
      steps[upload]?.if !== "always() && steps.nx-cache-trust.outputs.promotable == 'true'"
    )
      problems.push(`${name}: report must run always and upload only trusted entries`);
    if (
      steps.some(
        (step) =>
          step.uses?.startsWith("actions/cache") && /\.nx\/cache/.test(step.with?.path ?? ""),
      )
    )
      problems.push(`${name}: local Nx cache must not use Actions cache`);
    const stop = steps.find((step) => step.cancel === "nx-cache");
    if (JSON.stringify(stop) !== JSON.stringify({ cancel: "nx-cache" }))
      problems.push(`${name}: teardown must be a bare cancel step`);
  }
  if (
    release.on.workflow_dispatch ||
    !release.on.push?.branches?.includes(PROMOTION_BRANCH_PATTERN)
  )
    problems.push("promotion requires the dedicated real-push branch trigger");
  const releaseJob = release.jobs["release-plz"];
  if (releaseJob.permissions.actions !== "read") problems.push("promotion requires actions read");
  const steps = releaseJob.steps;
  const sidecar = steps.find((step) => step.id === "nx-cache");
  if (sidecar?.uses !== `op-nx/github-cache/start-cache-server@${OP_NX_COMMIT}`)
    problems.push("promotion sidecar must match the shared script pin");
  if (
    JSON.stringify(steps.find((step) => step.cancel === "nx-cache")) !==
    JSON.stringify({ cancel: "nx-cache" })
  )
    problems.push("promotion teardown must be a bare cancel step");
  const promote = steps.findIndex((step) => step.run?.includes("nx-remote-cache.mjs promote"));
  if (promote < 0) problems.push("release-plz must promote first");
  for (const [index, step] of steps.entries()) {
    if (
      step.run?.includes("scripts/scoped-release-plz.sh") ||
      step.run?.includes("scripts/prepare-release-pr.sh") ||
      step.run?.includes("scripts/enable-release-automerge.py")
    ) {
      if (
        index < promote ||
        !step.if?.includes("github.event_name == 'push' && github.ref == 'refs/heads/main'")
      )
        problems.push("every release-plz operation requires a main push after promotion");
    }
  }
  return problems;
}
test("the four existing CI jobs share the pinned cache and promotion precedes release-plz", () => {
  assert.deepEqual(cacheProblems(workflow("ci"), workflow("release-plz")), []);
});
test("cache shape refuses an untrusted writer, duplicate local store, invalid cancel steps and manual event", () => {
  for (const mutate of [
    (ci) => {
      delete ci.env.NX_BASE;
    },
    (ci) => {
      ci.env.NX_HEAD = "HEAD";
    },
    (ci) => {
      ci.jobs["test-linux"].env.NX_BASE = "HEAD~1";
    },
    (ci) => {
      ci.jobs.lint.steps[0].env = { NX_HEAD: "HEAD" };
    },

    (ci) => {
      ci.permissions.actions = "write";
    },
    (ci) => {
      delete ci.jobs.lint.permissions.actions;
    },
    (ci) => {
      ci.jobs.lint.steps.find((step) => step.id === "nx-cache").env.CACHE_READ_ONLY = "";
    },
    (ci) => {
      ci.jobs.lint.steps.push({ uses: "actions/cache@v6", with: { path: ".nx/cache" } });
    },
    (ci) => {
      ci.jobs.lint.steps.find((step) => step.cancel === "nx-cache")["continue-on-error"] = false;
    },
    (ci) => {
      ci.jobs.lint.steps.find((step) => step.cancel === "nx-cache").if = "always()";
    },
    (_ci, release) => {
      release.jobs["release-plz"].steps.find((step) => step.cancel === "nx-cache").if = "always()";
    },
    (_ci, release) => {
      release.jobs["release-plz"].steps.find((step) => step.cancel === "nx-cache")[
        "continue-on-error"
      ] = true;
    },
    (_ci, release) => {
      release.jobs["release-plz"].steps.find((step) => step.id === "nx-cache").uses =
        "op-nx/github-cache/start-cache-server@wrong";
    },
    (_ci, release) => {
      release.on.workflow_dispatch = {};
    },
  ]) {
    const ci = workflow("ci");
    const release = workflow("release-plz");
    mutate(ci, release);
    assert.ok(cacheProblems(ci, release).length);
  }
});

test("trust refuses writes for fork and Dependabot PRs, and permits a repository contributor", () => {
  const dir = tree();
  for (const [head, author, expected] of [
    ["contributor/fork", "contributor", "true"],
    ["nickderobertis/onetaskgraph", "dependabot[bot]", "true"],
    ["nickderobertis/onetaskgraph", "contributor", "false"],
  ]) {
    const event = join(dir, "event.json");
    const output = join(dir, "output");
    writeFileSync(
      event,
      JSON.stringify({
        pull_request: {
          base: { repo: { full_name: "nickderobertis/onetaskgraph" } },
          head: { repo: { full_name: head } },
          user: { login: author },
        },
      }),
    );
    writeFileSync(output, "");
    command(process.execPath, [join(root, "scripts/nx-remote-cache.mjs"), "trust"], dir, {
      GITHUB_EVENT_PATH: event,
      GITHUB_OUTPUT: output,
      GITHUB_REPOSITORY: "nickderobertis/onetaskgraph",
    });
    assert.equal(
      readFileSync(output, "utf8"),
      `read_only=${expected}\npromotable=${expected === "false"}\n`,
    );
  }
});

test("the version digest masks only the package version and keeps dependencies and feature inputs", () => {
  const dir = tree();
  mkdirSync(join(dir, "scripts"));
  copyFileSync(
    join(root, "scripts/nx-version-free-inputs.mjs"),
    join(dir, "scripts/nx-version-free-inputs.mjs"),
  );
  command("git", ["init", "--quiet"], dir);
  const manifest = (version, dependency = "1.0", feature = "[]") =>
    `[package]\nname = "fixture"\nversion = "${version}"\n[dependencies]\nserde = "${dependency}"\n[features]\ndefault = ${feature}\nextra = []\n`;
  const path = join(dir, "Cargo.toml");
  writeFileSync(path, manifest("1.0.0"));
  command("git", ["add", "."], dir);
  const digest = (keep = false) =>
    command(
      process.execPath,
      [
        "scripts/nx-version-free-inputs.mjs",
        "--ecosystem",
        "cargo",
        ...(keep ? ["--keep-version"] : []),
      ],
      dir,
    );
  const original = digest();
  const kept = digest(true);
  writeFileSync(path, manifest("1.0.1"));
  assert.equal(digest(), original);
  assert.notEqual(digest(true), kept);
  writeFileSync(path, manifest("1.0.1", "1.1"));
  assert.notEqual(digest(), original);
  writeFileSync(path, manifest("1.0.1", "1.0", '["extra"]'));
  assert.notEqual(digest(), original);
});

async function until(predicate) {
  for (let attempt = 0; !predicate(); attempt++) {
    assert.ok(attempt < 300, "the subprocess never became ready");
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
}
async function unusedPort() {
  const server = createServer();
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const port = server.address().port;
  await new Promise((resolve) => server.close(resolve));
  return port;
}
async function stop(process) {
  if (process.exitCode !== null || process.signalCode !== null) return;
  const exited = new Promise((resolve) => process.once("exit", resolve));
  process.kill("SIGTERM");
  await exited;
}
function envFile(file) {
  const value = Object.fromEntries(
    readFileSync(file, "utf8")
      .trim()
      .split("\n")
      .filter(Boolean)
      .map((line) => {
        const at = line.indexOf("=");
        assert.ok(
          at > 0 && /^[A-Za-z_][A-Za-z0-9_]*$/.test(line.slice(0, at)),
          `invalid environment record: ${line}`,
        );
        return [line.slice(0, at), line.slice(at + 1)];
      }),
  );
  if ("read_only" in value) {
    assert.deepEqual(Object.keys(value).sort(), ["promotable", "read_only", "token"]);
    for (const key of ["read_only", "promotable"])
      assert.ok(["true", "false"].includes(value[key]));
    assert.match(value.token, /^[a-f0-9]{64}$/);
  } else {
    assert.deepEqual(Object.keys(value).sort(), [
      "NX_SELF_HOSTED_REMOTE_CACHE_ACCESS_TOKEN",
      "NX_SELF_HOSTED_REMOTE_CACHE_SERVER",
    ]);
    assert.ok(loopback(value.NX_SELF_HOSTED_REMOTE_CACHE_SERVER));
    assert.match(value.NX_SELF_HOSTED_REMOTE_CACHE_ACCESS_TOKEN, /^[a-f0-9]{64}$/);
  }
  return value;
}

async function stopProxy(file) {
  if (!existsSync(file)) return;
  const proxy = jsonFile(
    file,
    (value) => object(value) && loopback(value.url) && text(value.secret),
    "loopback proxy address and secret",
  );
  try {
    await fetch(`${proxy.url}/stop`, { headers: { authorization: proxy.secret } });
  } catch (error) {
    // A CI report already stops its recording proxy before this fixture teardown.
    if (error.cause?.code !== "ECONNREFUSED") throw error;
  }
}

// Compare only fixture-supplied fields against observed GitHub responses. Scenario
// identities and outcomes vary; inventing a field or changing its type is refused.
function observedShape(observed, fixture, path = "response") {
  if (Array.isArray(fixture)) {
    assert.ok(Array.isArray(observed) && observed.length > 0, path);
    for (const item of fixture) observedShape(observed[0], item, `${path}[]`);
  } else if (object(fixture)) {
    assert.ok(object(observed), path);
    for (const [key, value] of Object.entries(fixture)) {
      assert.ok(Object.hasOwn(observed, key), `${path}.${key}: absent from recording`);
      observedShape(observed[key], value, `${path}.${key}`);
    }
  } else {
    assert.equal(typeof fixture, typeof observed, `${path}: changed recorded type`);
    assert.equal(fixture === null, observed === null, `${path}: changed nullability`);
  }
}
const instant = (value) => typeof value === "string" && Number.isFinite(Date.parse(value));
const positive = (value) => integer(value) && value > 0;
const repositoryIdentity = (value) => object(value) && text(value.full_name);
const commitIdentity = (value) => object(value) && sha(value.sha);
const prIdentity = (value) =>
  object(value) &&
  positive(value.number) &&
  commitIdentity(value.head) &&
  commitIdentity(value.base);
const recordedPromotion = jsonFile(
  join(root, ".github/cache-policy/tests/fixtures/github-promotion-recorded.json"),
  (value) =>
    object(value) &&
    object(value.provenance) &&
    Object.values(value.provenance).every(text) &&
    prIdentity(value.pull) &&
    repositoryIdentity(value.pull.head.repo) &&
    repositoryIdentity(value.pull.base.repo) &&
    text(value.pull.head.ref) &&
    text(value.pull.base.ref) &&
    text(value.pull.user?.login) &&
    instant(value.pull.merged_at) &&
    sha(value.pull.merge_commit_sha) &&
    object(value.run) &&
    positive(value.run.id) &&
    positive(value.run.run_attempt) &&
    sha(value.run.head_sha) &&
    text(value.run.event) &&
    text(value.run.status) &&
    text(value.run.conclusion) &&
    text(value.run.head_branch) &&
    repositoryIdentity(value.run.head_repository) &&
    positive(value.run.head_repository.id) &&
    Array.isArray(value.run.pull_requests) &&
    value.run.pull_requests.length > 0 &&
    value.run.pull_requests.every(prIdentity) &&
    instant(value.run.run_started_at) &&
    instant(value.run.updated_at) &&
    object(value.job) &&
    text(value.job.name) &&
    text(value.job.conclusion) &&
    Array.isArray(value.job.labels) &&
    value.job.labels.length > 0 &&
    value.job.labels.every(text) &&
    object(value.artifact) &&
    positive(value.artifact.id) &&
    text(value.artifact.name) &&
    typeof value.artifact.expired === "boolean" &&
    instant(value.artifact.created_at) &&
    object(value.artifact.workflow_run) &&
    positive(value.artifact.workflow_run.id) &&
    positive(value.artifact.workflow_run.head_repository_id) &&
    sha(value.artifact.workflow_run.head_sha) &&
    object(value.release) &&
    text(value.release.tag_name) &&
    typeof value.release.draft === "boolean" &&
    instant(value.release.published_at) &&
    commitIdentity(value.commit) &&
    object(value.compare) &&
    ["ahead", "behind", "identical", "diverged"].includes(value.compare.status),
  "recorded GitHub PR, run, job, artifact, release and commit projections",
);
test("promotion HTTP fixtures reject drift from the recorded GitHub response fields", () => {
  assert.throws(
    () => observedShape(recordedPromotion.pull, { number: "1" }),
    /changed recorded type/,
  );
  assert.throws(
    () => observedShape(recordedPromotion.run, { invented: true }),
    /absent from recording/,
  );
});

// Release selection is simulated; executions, task hashes, cache payloads and the
// promotion receipt below are produced by this journey's real Nx/sidecar/CLI runs.
function proveCacheBudget({ dir, workspace, env, pr, laterPr, routes }) {
  const repository = env.GITHUB_REPOSITORY;
  const prefix = `/repos/${repository}`;
  const end = new Date(Date.now() + 60_000).toISOString();
  const scope = "cache-promote/1";
  const sourcePr = structuredClone(pr);
  sourcePr.head.ref = "cache-journey-source";
  sourcePr.base.ref = scope;
  const releasePr = structuredClone(laterPr);
  releasePr.head.ref = "release-plz-cache-journey";
  releasePr.base.ref = scope;
  releasePr.merge_commit_sha = "5".repeat(40);
  releasePr.merged_at = new Date().toISOString();
  const reports = ["contributor", "replay", "later-pr"].map((name) => {
    const path = join(dir, name, "nx-cache-report");
    return {
      path,
      execution: executionFile(join(path, "executions.json")),
      manifest: manifestFile(join(path, "manifest.json")),
    };
  });
  const runs = reports.map(({ execution }, index) => {
    const selected = index === 2 ? releasePr : sourcePr;
    const times = [
      ...execution.tasks.map((task) => task.finished_at),
      ...execution.store_attempts.map((attempt) => attempt.attempted_at),
    ];
    return {
      id: execution.run_id,
      run_attempt: execution.attempt,
      head_sha: execution.head_sha,
      status: "completed",
      event: "pull_request",
      conclusion: "success",
      head_repository: { full_name: repository, id: 1 },
      pull_requests: [
        {
          number: selected.number,
          head: { sha: selected.head.sha },
          base: { sha: selected.base.sha },
        },
      ],
      run_started_at: execution.tasks[0].started_at,
      updated_at: new Date(Math.max(...times.map(Date.parse)) + 1).toISOString(),
    };
  });
  const receipt = jsonFile(
    join(dir, "promote-manual/nx-cache-promotion/receipt.json"),
    (value) =>
      object(value) &&
      typeof value.promoted_at === "string" &&
      Number.isFinite(Date.parse(value.promoted_at)) &&
      Array.isArray(value.attempts) &&
      value.attempts.every(object) &&
      Array.isArray(value.entries),
    "produced promotion receipt",
  );
  const promotion = {
    id: 4,
    head_sha: pr.merge_commit_sha,
    head_branch: scope,
    head_repository: { full_name: repository, id: 1 },
    event: "push",
    updated_at: end,
  };
  const artifact = (id, name, run, created) => ({
    id,
    name,
    expired: false,
    created_at: created,
    workflow_run: { id: run.id, head_sha: run.head_sha, head_repository_id: 1 },
  });
  const zip = (folder, filename) =>
    command(
      "python3",
      [
        "-c",
        "import pathlib,sys,zipfile\nroot=pathlib.Path(sys.argv[1])\nwith zipfile.ZipFile(sys.argv[2], 'w') as archive:\n for file in root.rglob('*'):\n  if file.is_file(): archive.write(file,file.relative_to(root))",
        folder,
        join(dir, filename),
      ],
      workspace,
    );
  const baseline = structuredClone(routes);
  baseline[`${prefix}/releases`] = [
    { tag_name: "v-cache-journey-simulated", draft: false, published_at: end },
  ];
  baseline[`${prefix}/commits/v-cache-journey-simulated`] = { sha: releasePr.merge_commit_sha };
  baseline[`${prefix}/commits`] = [
    { sha: sourcePr.merge_commit_sha },
    { sha: releasePr.merge_commit_sha },
  ];
  baseline[`${prefix}/commits/${sourcePr.merge_commit_sha}/pulls`] = [sourcePr];
  baseline[`${prefix}/commits/${sourcePr.merge_commit_sha}`] = { sha: sourcePr.merge_commit_sha };
  baseline[`${prefix}/commits/${releasePr.merge_commit_sha}/pulls`] = [releasePr];
  baseline[`${prefix}/compare/${sourcePr.merge_commit_sha}...${releasePr.base.sha}`] = {
    status: "identical",
  };
  baseline[`${prefix}/actions/workflows/ci.yml/runs`] = { workflow_runs: runs };
  baseline[`${prefix}/actions/workflows/release-plz.yml/runs`] = { workflow_runs: [promotion] };
  baseline[`${prefix}/actions/runs/4/artifacts`] = {
    artifacts: [
      artifact(40, `nx-cache-promotion-${pr.merge_commit_sha}`, promotion, receipt.promoted_at),
    ],
  };
  baseline[`${prefix}/actions/artifacts/40/zip`] = { fixture_file: "budget-receipt.zip" };
  for (const [index, run] of runs.entries()) {
    const id = 10 + index;
    baseline[`${prefix}/actions/runs/${run.id}/attempts/${run.run_attempt}`] = run;
    baseline[`${prefix}/actions/runs/${run.id}/attempts/${run.run_attempt}/jobs`] = {
      jobs: [
        {
          name: "lint",
          conclusion: "success",
          labels: [process.platform === "darwin" ? "macos-latest" : "ubuntu-latest"],
        },
      ],
    };
    baseline[`${prefix}/actions/runs/${run.id}/artifacts`] = {
      artifacts: [artifact(id, "nx-cache-entries-lint", run, run.updated_at)],
    };
    baseline[`${prefix}/actions/artifacts/${id}/zip`] = {
      fixture_file: `budget-report-${index}.zip`,
    };
  }
  observedShape(recordedPromotion.pull, sourcePr);
  observedShape(recordedPromotion.pull, releasePr);
  for (const candidate of runs) observedShape(recordedPromotion.run, candidate);
  observedShape(recordedPromotion.release, baseline[`${prefix}/releases`][0]);
  for (const commit of baseline[`${prefix}/commits`])
    observedShape(recordedPromotion.commit, commit);
  observedShape(
    recordedPromotion.compare,
    baseline[`${prefix}/compare/${sourcePr.merge_commit_sha}...${releasePr.base.sha}`],
  );
  const document = parse(readFileSync(join(root, "budgets.yaml"), "utf8"));
  assert.ok(
    object(document) &&
      document.schema_version === 1 &&
      Array.isArray(document.budgets) &&
      document.budgets.every((item) => object(item) && text(item.id)),
  );
  const budget = document.budgets.find((entry) => entry.id === "onetaskgraph-nx-cache-use");
  assert.ok(budget);
  const specification = join(workspace, "cache-budget.json");
  writeFileSync(
    specification,
    JSON.stringify({ schema_version: document.schema_version, budgets: [budget] }),
  );
  for (const scenario of [
    "replayed",
    "evicted",
    "failed-store",
    "throttled-store",
    "duplicate-run",
    "later-push",
    "never-promoted",
    "failed",
    "throttled",
    "changed-hash",
    "local-hit",
    "missing-job",
    "expired",
    "missing-artifact",
    "untrusted-receipt",
    "invalid-receipt",
    "zero-eligible",
  ]) {
    const answers = structuredClone(baseline);
    const documents = reports.map(({ execution, manifest }) => ({
      execution: structuredClone(execution),
      manifest: structuredClone(manifest),
    }));
    const currentReceipt = structuredClone(receipt);
    const current = documents[2].execution.tasks[0];
    if (["evicted", "never-promoted", "failed", "throttled"].includes(scenario)) {
      current.result = "executed";
      current.remote_status = 404;
    }
    if (scenario === "never-promoted")
      answers[`${prefix}/actions/workflows/release-plz.yml/runs`].workflow_runs = [];
    if (["failed", "throttled"].includes(scenario)) {
      currentReceipt.entries = [];
      for (const attempt of currentReceipt.attempts) {
        attempt.status = "failed";
        attempt.http_status = scenario === "throttled" ? 429 : 500;
      }
    }
    if (scenario === "changed-hash") {
      current.hash = "deadbeef";
      current.result = "executed";
      current.remote_status = 404;
    }
    if (scenario === "local-hit") current.result = "local_hit";
    if (current.result !== "remote_hit") {
      documents[2].manifest.entries = [];
      documents[2].manifest.stored_bytes = 0;
    }
    if (["failed-store", "throttled-store"].includes(scenario)) {
      const status = scenario === "failed-store" ? "failed" : "throttled";
      const http = status === "failed" ? 500 : 429;
      for (const attempt of documents[0].execution.store_attempts) {
        attempt.status = status;
        attempt.http_status = http;
      }
      documents[0].manifest.entries = [];
      documents[0].manifest.stored_bytes = 0;
      const retry = documents[1].execution;
      retry.tasks[0].result = "executed";
      retry.tasks[0].remote_status = http;
      retry.store_attempts = documents[0].execution.store_attempts.map((attempt) => ({
        ...attempt,
        attempted_at: retry.tasks[0].finished_at,
      }));
      documents[1].manifest.entries = [];
      documents[1].manifest.stored_bytes = 0;
      currentReceipt.entries = [];
      for (const attempt of currentReceipt.attempts) {
        attempt.status = status;
        attempt.http_status = http;
      }
    }
    if (scenario === "duplicate-run")
      answers[`${prefix}/actions/workflows/ci.yml/runs`].workflow_runs.push(
        structuredClone(runs[1]),
      );
    if (scenario === "later-push") {
      const head = "6".repeat(40);
      answers[`${prefix}/commits/${sourcePr.merge_commit_sha}/pulls`][0].head.sha = head;
      const changed = answers[`${prefix}/actions/runs/2/attempts/1`];
      changed.head_sha = head;
      changed.pull_requests[0].head.sha = head;
      const listing = answers[`${prefix}/actions/workflows/ci.yml/runs`].workflow_runs.find(
        (item) => item.id === 2,
      );
      listing.head_sha = head;
      listing.pull_requests[0].head.sha = head;
      answers[`${prefix}/actions/runs/2/artifacts`].artifacts[0].workflow_run.head_sha = head;
      documents[1].execution.head_sha = head;
      documents[1].execution.selection.head_sha = head;
    }
    if (scenario === "missing-job")
      answers[`${prefix}/actions/runs/3/attempts/1/jobs`].jobs.push({
        name: "test-linux",
        conclusion: "success",
        labels: ["ubuntu-latest"],
      });
    if (scenario === "expired")
      answers[`${prefix}/actions/runs/3/artifacts`].artifacts[0].expired = true;
    if (scenario === "missing-artifact")
      answers[`${prefix}/actions/runs/3/artifacts`].artifacts = [];
    if (scenario === "untrusted-receipt") currentReceipt.repository = "untrusted/repository";
    if (scenario === "invalid-receipt") currentReceipt.source_run_id = "2";
    if (scenario === "zero-eligible") {
      answers[`${prefix}/actions/workflows/release-plz.yml/runs`].workflow_runs = [];
      for (const doc of documents) {
        doc.execution.store_attempts = [];
        doc.manifest.entries = [];
        doc.manifest.stored_bytes = 0;
        for (const task of doc.execution.tasks) task.result = "local_hit";
      }
    }
    for (const [index, record] of reports.entries()) {
      const folder = join(dir, `budget-${scenario}-${index}`);
      cpSync(record.path, folder, { recursive: true });
      writeFileSync(join(folder, "executions.json"), JSON.stringify(documents[index].execution));
      writeFileSync(join(folder, "manifest.json"), JSON.stringify(documents[index].manifest));
      zip(folder, `budget-report-${index}.zip`);
    }
    const receiptFolder = join(dir, `budget-${scenario}-receipt`);
    mkdirSync(receiptFolder);
    writeFileSync(join(receiptFolder, "receipt.json"), JSON.stringify(currentReceipt));
    zip(receiptFolder, "budget-receipt.zip");
    writeFileSync(join(dir, "routes.json"), JSON.stringify(answers));
    const result = spawnSync(
      process.execPath,
      [join(root, "node_modules/@onebudgetspec/cli/bin/onebudgetspec.js"), "check", specification],
      {
        cwd: workspace,
        encoding: "utf8",
        timeout: 120_000,
        env: {
          ...process.env,
          ...env,
          GITHUB_REF_NAME: "v-cache-journey-simulated",
          RUNNER_TEMP: join(dir, `budget-${scenario}-temp`),
          NODE_TEST_CONTEXT: undefined,
        },
      },
    );
    const output = result.stdout + result.stderr;
    if (
      [
        "missing-job",
        "expired",
        "missing-artifact",
        "untrusted-receipt",
        "invalid-receipt",
        "zero-eligible",
      ].includes(scenario)
    ) {
      assert.notEqual(result.status, 0, `${scenario}: ${output}`);
      assert.doesNotMatch(output, /aggregate: \d+\/\d+ remote hits/, `${scenario}: ${output}`);
    } else {
      const expected = [
        "replayed",
        "duplicate-run",
        "later-push",
        "changed-hash",
        "local-hit",
      ].includes(scenario)
        ? 100
        : 50;
      assert.match(output, new RegExp(`actual ${expected} percent`), `${scenario}: ${output}`);
      assert.match(
        output,
        new RegExp(
          `aggregate: ${expected === 50 ? "1/2" : ["replayed", "duplicate-run", "later-push"].includes(scenario) ? "2/2" : "1/1"} remote hits`,
        ),
        `${scenario}: ${output}`,
      );
      if (["evicted", "never-promoted", "failed", "throttled"].includes(scenario))
        assert.match(output, new RegExp(scenario.replaceAll("-", "_")), output);
      if (scenario === "replayed")
        console.info(
          `Simulated release selection, real journey telemetry: ${output.split("\n")[0]}`,
        );
      if (scenario === "local-hit") assert.match(output, /skipped local hits: 1/);
      if (["failed-store", "throttled-store"].includes(scenario))
        assert.match(output, new RegExp(scenario.split("-")[0]));
      if (["duplicate-run", "later-push"].includes(scenario))
        assert.match(output, /same-PR retry\/push: 1\/1/);
    }
  }
}

async function provePromotion({ dir, workspace, action, sourceEnv }) {
  const repository = "nickderobertis/onetaskgraph";
  const reportDir = join(dir, "replay/nx-cache-report");
  const execution = executionFile(join(reportDir, "executions.json"));
  const pr = {
    number: 1,
    head: { sha: execution.head_sha, repo: { full_name: repository } },
    base: { sha: execution.selection.base_sha, repo: { full_name: repository } },
    user: { login: "contributor" },
    merged_at: new Date().toISOString(),
    merge_commit_sha: command("git", ["rev-parse", "705faa9"], workspace),
  };
  const run = {
    id: 2,
    run_attempt: 1,
    head_sha: pr.head.sha,
    event: "pull_request",
    conclusion: "success",
    head_repository: { full_name: repository, id: 1 },
    pull_requests: [{ number: pr.number, head: { sha: pr.head.sha }, base: { sha: pr.base.sha } }],
    run_started_at: execution.tasks[0].started_at,
    updated_at: new Date().toISOString(),
  };
  command(
    "python3",
    [
      "-c",
      "import pathlib,sys,zipfile\nwith zipfile.ZipFile(sys.argv[2], 'w') as archive:\n for file in pathlib.Path(sys.argv[1]).iterdir():\n  if file.is_file(): archive.write(file, file.name)",
      reportDir,
      join(dir, "source.zip"),
    ],
    workspace,
  );
  const routes = {
    [`/repos/${repository}/pulls/1`]: pr,
    [`/repos/${repository}/commits/${pr.merge_commit_sha}/pulls`]: [pr],
    [`/repos/${repository}/actions/workflows/ci.yml/runs`]: { workflow_runs: [run] },
    [`/repos/${repository}/actions/runs/2/attempts/1`]: run,
    [`/repos/${repository}/actions/runs/2/attempts/1/jobs`]: {
      jobs: [
        {
          name: "lint",
          conclusion: "success",
          labels: [
            process.platform === "darwin"
              ? "macos"
              : process.platform === "win32"
                ? "windows"
                : "ubuntu",
          ],
        },
      ],
    },
    [`/repos/${repository}/actions/runs/2/artifacts`]: {
      artifacts: [
        {
          id: 1,
          name: "nx-cache-entries-lint",
          expired: false,
          created_at: run.updated_at,
          workflow_run: { id: 2, head_sha: pr.head.sha, head_repository_id: 1 },
        },
      ],
    },
    [`/repos/${repository}/actions/artifacts/1/zip`]: { fixture_file: "source.zip" },
  };
  observedShape(recordedPromotion.pull, pr);
  observedShape(recordedPromotion.run, run);
  observedShape(
    recordedPromotion.job,
    routes[`/repos/${repository}/actions/runs/2/attempts/1/jobs`].jobs[0],
  );
  observedShape(
    recordedPromotion.artifact,
    routes[`/repos/${repository}/actions/runs/2/artifacts`].artifacts[0],
  );
  const writeRoutes = () => writeFileSync(join(dir, "routes.json"), JSON.stringify(routes));
  const releaseSteps = workflow("release-plz").jobs["release-plz"].steps;
  const promote = releaseSteps.find((step) => step.run?.includes("nx-remote-cache.mjs promote"));
  const release = releaseSteps.find((step) =>
    step.run?.includes("scoped-release-plz.sh run release"),
  );
  async function session(name, ref, eventName, event, steps, body) {
    const runDir = join(dir, name);
    mkdirSync(runDir);
    const eventPath = join(runDir, "event.json");
    writeFileSync(eventPath, JSON.stringify(event));
    const port = await unusedPort();
    const env = {
      ...sourceEnv,
      GITHUB_EVENT_NAME: eventName,
      GITHUB_EVENT_PATH: eventPath,
      GITHUB_REF: ref,
      GITHUB_REF_NAME: ref.replace("refs/heads/", ""),
      GITHUB_SHA: pr.merge_commit_sha,
      GITHUB_RUN_ID: "3",
      GITHUB_JOB: eventName === "push" ? "release-plz" : "lint",
      ACTIONS_RUNTIME_TOKEN: ref,
      RUNNER_TEMP: runDir,
      GITHUB_ENV: join(runDir, "env"),
      GITHUB_OUTPUT: join(runDir, "outputs"),
      GITHUB_STEP_SUMMARY: join(runDir, "summary"),
      INPUT_PORT: String(port),
      NX_SELF_HOSTED_REMOTE_CACHE_SERVER: `http://127.0.0.1:${port}`,
    };
    if (eventName === "push") {
      delete env.NX_BASE;
      delete env.NX_HEAD;
    } else
      for (const [key, expression] of Object.entries(workflow("ci").env))
        env[key] = evaluate(expression, { github: { event } });
    const trust = steps.find((step) => step.id === "nx-cache-trust");
    const sidecar = steps.find((step) => step.id === "nx-cache");
    const ready = steps.find((step) => step.run?.endsWith("nx-remote-cache.mjs ready"));
    command("bash", ["-eo", "pipefail", "-c", trust.run], workspace, env);
    const outputs = envFile(env.GITHUB_OUTPUT);
    env.CACHE_READ_ONLY = evaluate(sidecar.env.CACHE_READ_ONLY, {
      steps: { "nx-cache-trust": { outputs } },
    });
    env.NX_SELF_HOSTED_REMOTE_CACHE_ACCESS_TOKEN = outputs.token;
    const sidecarProcess = spawn(process.execPath, [join(action, "start-cache-server/index.js")], {
      cwd: workspace,
      env: { ...process.env, ...env },
      stdio: "ignore",
    });
    try {
      command("bash", ["-eo", "pipefail", "-c", ready.run], workspace, env);
      Object.assign(env, envFile(env.GITHUB_ENV));
      await body(env, runDir);
    } finally {
      try {
        await stopProxy(join(runDir, "nx-cache-recording", env.GITHUB_JOB, "proxy.json"));
      } finally {
        await stop(sidecarProcess);
      }
    }
  }
  for (const reason of ["fork", "dependabot", "gone", "merged", "manual"]) {
    const sourcePr = structuredClone(pr);
    if (reason === "fork") sourcePr.head.repo.full_name = "contributor/fork";
    if (reason === "dependabot") sourcePr.user.login = "dependabot[bot]";
    routes[`/repos/${repository}/pulls/1`] = sourcePr;
    routes[`/repos/${repository}/commits/${pr.merge_commit_sha}/pulls`] = [sourcePr];
    const artifactRoute = `/repos/${repository}/actions/runs/2/artifacts`;
    const artifacts = routes[artifactRoute];
    if (reason === "gone") routes[artifactRoute] = { artifacts: [] };
    writeRoutes();
    const ref = reason === "manual" ? "refs/heads/cache-promote/1" : "refs/heads/main";
    await session(
      `promote-${reason}`,
      ref,
      "push",
      { ref, repository: { full_name: repository, default_branch: "main" } },
      releaseSteps,
      async (env, runDir) => {
        command("bash", ["-eo", "pipefail", "-c", promote.run], workspace, env);
        const summary = readFileSync(env.GITHUB_STEP_SUMMARY, "utf8");
        const receiptPath = join(runDir, "nx-cache-promotion/receipt.json");
        if (["merged", "manual"].includes(reason)) {
          assert.match(summary, /Nx cache promotion: 1 promoted/);
          const receipt = jsonFile(
            receiptPath,
            (value) =>
              object(value) &&
              integer(value.source_run_id) &&
              text(value.scope) &&
              Array.isArray(value.entries) &&
              value.entries.every(
                (entry) =>
                  object(entry) &&
                  text(entry.hash) &&
                  text(entry.task) &&
                  ["linux", "darwin", "win32"].includes(entry.os),
              ),
            "promotion receipt identity and entries",
          );
          assert.equal(receipt.source_run_id, 2);
          assert.equal(receipt.entries.length, execution.tasks.length);
          assert.equal(receipt.scope, ref);
        } else {
          assert.match(summary, /promotion skipped/);
          assert.match(
            summary,
            new RegExp(
              { fork: "fork head", dependabot: "Dependabot author", gone: "missing artifact" }[
                reason
              ],
            ),
          );
          assert.equal(existsSync(receiptPath), false);
        }
        // Only release-plz's executable is doubled, at the existing wrapper's tool boundary.
        env.ONETASKGRAPH_TOOLS_HOME = join(runDir, "tools");
        env.RELEASE_BOUNDARY_LOG = join(runDir, "released");
        const pin = command("bash", ["scripts/scoped-release-plz.sh", "pin"], workspace, env);
        const binary = command("bash", ["scripts/scoped-release-plz.sh", "path"], workspace, env);
        assert.match(pin, /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/);
        assert.ok(
          isAbsolute(binary) &&
            binary === resolve(binary) &&
            binary.startsWith(`${resolve(env.ONETASKGRAPH_TOOLS_HOME)}${sep}`),
          "release-plz executable must remain inside the fixture tools directory",
        );
        mkdirSync(dirname(binary), { recursive: true });
        writeFileSync(
          binary,
          `#!/usr/bin/env bash\nif [ "$1" = --version ]; then echo 'release-plz ${pin}'; else echo released >> "$RELEASE_BOUNDARY_LOG"; fi\n`,
        );
        chmodSync(binary, 0o755);
        const enabled = evaluate(release.if, {
          github: { event_name: env.GITHUB_EVENT_NAME, ref },
          steps: { guard: { outputs: { enabled: "true" } } },
        });
        if (enabled) command("bash", ["-eo", "pipefail", "-c", release.run], workspace, env);
        assert.equal(existsSync(env.RELEASE_BOUNDARY_LOG), reason !== "manual");
      },
    );
    routes[artifactRoute] = artifacts;
  }
  writeFileSync(
    join(dir, "bases.json"),
    JSON.stringify({ "refs/pull/2/merge": "refs/heads/cache-promote/1" }),
  );
  const laterPr = structuredClone(pr);
  laterPr.number = 2;
  laterPr.base.sha = pr.merge_commit_sha;
  await session(
    "later-pr",
    "refs/pull/2/merge",
    "pull_request",
    { pull_request: laterPr },
    workflow("ci").jobs.lint.steps,
    async (env, runDir) => {
      rmSync(join(workspace, ".nx"), { recursive: true, force: true });
      command(
        process.execPath,
        [
          "node_modules/nx/dist/bin/nx.js",
          "run",
          "github-workflows:ci-jobs",
          "--excludeTaskDependencies",
          "--outputStyle=static",
        ],
        workspace,
        env,
      );
      const report = workflow("ci").jobs.lint.steps.find((step) =>
        step.run?.includes("nx-remote-cache.mjs report"),
      );
      command("bash", ["-eo", "pipefail", "-c", report.run], workspace, env);
      const evidence = executionFile(join(runDir, "nx-cache-report/executions.json"));
      assert.ok(
        evidence.tasks.length > 0 && evidence.tasks.every((task) => task.result === "remote_hit"),
      );
    },
  );
  proveCacheBudget({ dir, workspace, env: sourceEnv, pr, laterPr, routes });
}

test("CI and release-plz promote a replay-only run, share it with a later PR, and report cache-use with simulated release selection", {
  timeout: 300_000,
}, async () => {
  const dir = tree();
  const workspace = join(dir, "workspace");
  command("git", ["clone", "--quiet", "--shared", root, workspace], dir);
  // Include this dispatch's working changes in the fixture before its commit exists.
  for (const file of command("git", ["diff", "--name-only", "-z", "HEAD"], root)
    .split("\0")
    .filter(Boolean)) {
    assert.ok(
      !file.startsWith("/") && !file.split("/").some((part) => part === ".." || part === ""),
      "expected a relative Git path",
    );
    const destination = join(workspace, file);
    mkdirSync(dirname(destination), { recursive: true });
    if (existsSync(join(root, file))) copyFileSync(join(root, file), destination);
    else rmSync(destination, { force: true });
  }
  symlinkSync(join(root, "node_modules"), join(workspace, "node_modules"), "dir");
  symlinkSync(join(root, ".github/node_modules"), join(workspace, ".github/node_modules"), "dir");
  const action = join(dir, "action");
  command("git", ["clone", "--quiet", "https://github.com/op-nx/github-cache.git", action], dir);
  command("git", ["checkout", "--quiet", OP_NX_COMMIT], action);
  const github = spawn(
    "python3",
    [join(root, ".github/cache-policy/tests/fixtures/nx-cache-http.py"), dir],
    {
      stdio: ["pipe", "ignore", "inherit"],
    },
  );
  try {
    await until(() => existsSync(join(dir, "url")) || github.exitCode !== null);
    const url = readFileSync(join(dir, "url"), "utf8");
    assert.ok(loopback(url), "fixture server must use loopback HTTP");
    let sourceEnv;
    const steps = workflow("ci").jobs.lint.steps;
    const trust = steps.find((step) => step.id === "nx-cache-trust");
    const sidecar = steps.find((step) => step.id === "nx-cache");
    const ready = steps.find((step) => step.run?.endsWith("nx-remote-cache.mjs ready"));
    const report = steps.find((step) => step.run?.includes("nx-remote-cache.mjs report"));
    for (const [identity, expectedReadOnly] of [
      ["fork", true],
      ["dependabot", true],
      ["throttled", false],
      ["contributor", false],
      ["replay", false],
    ]) {
      const control = await fetch(`${url}control`, {
        method: "POST",
        body: JSON.stringify({ mode: identity === "throttled" ? "throttled" : "live" }),
      });
      assert.ok(control.ok);
      const runDir = join(dir, identity);
      mkdirSync(runDir);
      const event = join(runDir, "event.json");
      const record = ["contributor", "replay", "throttled"].includes(identity)
        ? {
            repository: "nickderobertis/onetaskgraph",
            event: {
              pull_request: {
                number: 1,
                head: { sha: "1".repeat(40), repo: { full_name: "nickderobertis/onetaskgraph" } },
                base: { sha: "2".repeat(40), repo: { full_name: "nickderobertis/onetaskgraph" } },
                user: { login: "contributor" },
              },
            },
          }
        : eventFile(join(root, `.github/cache-policy/tests/fixtures/nx-cache-${identity}.json`));
      writeFileSync(event, JSON.stringify(record.event));
      const pr = record.event.pull_request;
      const port = await unusedPort();
      const env = {
        GITHUB_ACTIONS: "true",
        GITHUB_EVENT_NAME: "pull_request",
        GITHUB_REPOSITORY: record.repository,
        GITHUB_REF: `refs/pull/${pr.number}/merge`,
        GITHUB_SHA: "3".repeat(40),
        GITHUB_EVENT_PATH: event,
        GITHUB_WORKSPACE: workspace,
        GITHUB_JOB: "lint",
        GITHUB_RUN_ID: identity === "replay" ? "2" : "1",
        GITHUB_RUN_ATTEMPT: "1",
        GITHUB_SERVER_URL: "https://github.com",
        GITHUB_TOKEN: "test-token",
        GH_TOKEN: "test-token",
        GITHUB_API_URL: url.replace(/\/$/, ""),
        ACTIONS_RUNTIME_TOKEN: `refs/pull/${pr.number}/merge`,
        ACTIONS_CACHE_SERVICE_V2: "true",
        ACTIONS_RESULTS_URL: url,
        RUNNER_TEMP: runDir,
        GITHUB_ENV: join(runDir, "env"),
        GITHUB_OUTPUT: join(runDir, "outputs"),
        GITHUB_STEP_SUMMARY: join(runDir, "summary"),
        INPUT_PORT: String(port),
        NX_SELF_HOSTED_REMOTE_CACHE_SERVER: `http://127.0.0.1:${port}`,
        NX_DAEMON: "false",
        NX_ISOLATE_PLUGINS: "false",
      };
      for (const [name, expression] of Object.entries(workflow("ci").env))
        env[name] = evaluate(expression, { github: { event: record.event } });
      command("bash", ["-eo", "pipefail", "-c", trust.run], workspace, env);
      const outputs = envFile(env.GITHUB_OUTPUT);
      assert.equal(outputs.read_only, String(expectedReadOnly));
      env.CACHE_READ_ONLY = evaluate(sidecar.env.CACHE_READ_ONLY, {
        steps: { "nx-cache-trust": { outputs } },
      });
      env.NX_SELF_HOSTED_REMOTE_CACHE_ACCESS_TOKEN = outputs.token;
      const sidecarProcess = spawn(
        process.execPath,
        [join(action, "start-cache-server/index.js")],
        {
          cwd: workspace,
          env: { ...globalThis.process.env, ...env },
          stdio: ["ignore", "pipe", "pipe"],
        },
      );
      let logs = "";
      sidecarProcess.stdout.on("data", (chunk) => {
        logs += chunk;
      });
      sidecarProcess.stderr.on("data", (chunk) => {
        logs += chunk;
      });
      try {
        command("bash", ["-eo", "pipefail", "-c", ready.run], workspace, env);
        assert.ok(existsSync(env.GITHUB_ENV), logs);
        Object.assign(env, envFile(env.GITHUB_ENV));
        // Nx 23 keeps cache records in workspace-data as well as cache payloads.
        rmSync(join(workspace, ".nx"), { recursive: true, force: true });
        const result = command(
          process.execPath,
          [
            "node_modules/nx/dist/bin/nx.js",
            "run",
            "github-workflows:ci-jobs",
            "--excludeTaskDependencies",
            "--outputStyle=static",
          ],
          workspace,
          env,
        );
        assert.match(result, /the finished workflows hold the four jobs/);
        command("bash", ["-eo", "pipefail", "-c", report.run], workspace, env);
        const manifest = manifestFile(join(runDir, "nx-cache-report/manifest.json"));
        assert.equal(
          manifest.entries.length > 0,
          !expectedReadOnly && identity !== "throttled",
          JSON.stringify(manifest),
        );
        assert.equal(
          manifest.stored_bytes > 0,
          !expectedReadOnly && !["replay", "throttled"].includes(identity),
        );
        if (identity === "replay")
          assert.ok(manifest.entries.every((entry) => entry.source === "replayed"));
        const executions = executionFile(join(runDir, "nx-cache-report/executions.json"));
        assert.deepEqual(executions.selection, {
          mode: "affected",
          base_sha: pr.base.sha,
          head_sha: pr.head.sha,
        });
        if (identity === "throttled")
          assert.ok(
            executions.store_attempts.length > 0 &&
              executions.store_attempts.every((attempt) => attempt.status !== "stored"),
          );
        if (identity === "replay") sourceEnv = { ...env };
        assert.ok(
          executions.tasks.some(
            (task) =>
              task.task === "github-workflows:ci-jobs" &&
              task.result === (identity === "replay" ? "remote_hit" : "executed"),
          ),
        );
      } finally {
        await stop(sidecarProcess);
      }
    }
    await provePromotion({ dir, workspace, action, sourceEnv });
  } finally {
    const exited = new Promise((resolve) => github.once("exit", resolve));
    if (github.exitCode === null) {
      github.stdin.end();
      await exited;
    }
  }
  const entries = jsonFile(
    join(dir, "entries.json"),
    (value) =>
      Array.isArray(value) &&
      value.every(
        (entry) =>
          object(entry) &&
          text(entry.scope) &&
          object(entry.entry) &&
          text(entry.entry.key) &&
          text(entry.entry.version),
      ),
    "scoped cache entries",
  );
  for (const scope of ["refs/pull/1/merge", "refs/heads/main", "refs/heads/cache-promote/1"])
    assert.ok(entries.some((entry) => entry.scope === scope));
  const calls = jsonFile(
    join(dir, "calls.json"),
    (value) =>
      Array.isArray(value) &&
      value.every(
        (call) =>
          object(call) &&
          text(call.method) &&
          typeof call.scope === "string" &&
          typeof call.key === "string" &&
          typeof call.version === "string",
      ),
    "cache request records",
  );
  assert.equal(calls.filter(({ method }) => method === "FinalizeCacheEntryUpload").length, 3);
  assert.equal(
    calls.some(({ method }) => method === "DELETE"),
    false,
  );
});
