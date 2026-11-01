import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, before, test } from "node:test";
import { fileURLToPath } from "node:url";
import { parse, stringify } from "yaml";

// The release tail, as `publish` runs it: the job's own install step and its last step, read
// from release.yml and run as written, with the real onevcs and onebudgetspec they install, the
// real jq and the copied figures script, over the root budgets.yaml. GitHub is the one boundary
// stood in for: `ONEVCS_GH` — onevcs's seam for replaying recorded responses — names a double
// serving the responses recorded, read-only, from two of this repository's own releases and the
// runs that carried them (fixtures/release-tail/): v0.12.13, and v0.13.0, whose release commit
// carries a push run concluded skipped. Every figure is held to the one this test computes from
// those recordings, and every read count to what the double itself served.

const root = fileURLToPath(new URL("../../../", import.meta.url));
// The onevcs release this repository adopts: one whose `onevcs host` answers schema_version 1.
const ADOPTED_ONEVCS = "0.57.4";
const REPOSITORY = "nickderobertis/onetaskgraph";
// Each recorded release: its recordings, and the Release run whose publish job runs the tail.
const RELEASES = {
  "v0.12.13": { dir: "recordings/", run: 41026720026 },
  "v0.13.0": { dir: "recordings-v0.13.0/", run: 41252264998 },
};
const JOB_TOKEN = `\${{ github.token }}`;
const TAIL_IDS = [
  "onetaskgraph-ci-check-time",
  "onetaskgraph-cd-job-time",
  "onetaskgraph-release-cycle-time",
  "onetaskgraph-ci-job-count",
  "onetaskgraph-cd-job-count",
  "onetaskgraph-release-read-requests",
  "onetaskgraph-pr-read-requests",
];

// The CD figures leave out the runs of the one workflow their commands name with
// --exclude-workflow: the main-push Rust cache warmer, which builds and delivers nothing.
const TAIL_BUDGETS = () =>
  parse(readFileSync(join(root, "budgets.yaml"), "utf8")).budgets.filter((budget) =>
    TAIL_IDS.includes(budget.id),
  );
const excludedBy = (command) =>
  command.flatMap((arg, n) => (command[n - 1] === "--exclude-workflow" ? [arg] : []));
const WARMER = parse(
  readFileSync(join(root, ".github/workflows/rust-cache-warm.yml"), "utf8"),
).name;
const EXCLUDED = [WARMER];

const releaseWorkflow = () =>
  parse(readFileSync(join(root, ".github/workflows/release.yml"), "utf8"));
const rootPackage = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));

// Every way the tail could stop being a report nothing reads, or stop running the onevcs this
// repository adopted.
export function tailProblems(workflow) {
  const problems = [];
  const publish = workflow.jobs?.publish;
  if (!publish) return ["release.yml has no publish job"];
  const steps = publish.steps ?? [];
  const last = steps.at(-1);
  if (last?.run?.trim() !== "onebudgetspec check budgets.yaml --label release-tail")
    problems.push(
      "publish's last step is not `onebudgetspec check budgets.yaml --label release-tail`",
    );
  if (last?.["continue-on-error"] !== undefined)
    problems.push("the release tail carries continue-on-error");
  if (last?.env?.GH_TOKEN !== JOB_TOKEN)
    problems.push("the release tail is not handed the job's token as GH_TOKEN");
  const tailIndex = steps.findIndex((step) =>
    /onebudgetspec check [^\n]*--label release-tail/.test(step.run ?? ""),
  );
  if (tailIndex !== -1 && tailIndex !== steps.length - 1)
    problems.push("the release tail is not publish's last step");
  // Nothing reads the tail's outcome: no step after it, no job after publish, no expression
  // naming the step or the job.
  for (const [id, job] of Object.entries(workflow.jobs)) {
    const needs = typeof job.needs === "string" ? [job.needs] : (job.needs ?? []);
    if (needs.includes("publish"))
      problems.push(`${id} needs publish, so it reads the tail's outcome`);
  }
  const text = JSON.stringify(workflow);
  if (last?.id && new RegExp(`steps\\.${last.id}\\b`).test(text))
    problems.push("a step reads the tail's outcome");
  if (/needs\.publish\b/.test(text)) problems.push("an expression reads publish's result");
  const permissions = publish.permissions ?? {};
  for (const [scope, level] of Object.entries({
    actions: "read",
    contents: "read",
    "pull-requests": "read",
  }))
    if (permissions[scope] !== level) problems.push(`publish does not grant ${scope}: ${level}`);
  const install = steps.slice(0, -1).find((step) => /\bonevcs-cli@/.test(step.run ?? ""));
  if (!install) problems.push("no step of publish before its last installs the onevcs CLI");
  else {
    const version = install.run.match(/\bonevcs-cli@(\S+)/)[1];
    if (version !== ADOPTED_ONEVCS)
      problems.push(`publish installs onevcs-cli@${version}, not the adopted ${ADOPTED_ONEVCS}`);
    const cli = install.run.match(/@onebudgetspec\/cli@(\S+)/)?.[1];
    if (cli !== rootPackage.devDependencies["@onebudgetspec/cli"])
      problems.push(`publish installs @onebudgetspec/cli@${cli}, not package.json's pin`);
  }
  return problems;
}

test("publish ends in the release tail, reading-only and read by nothing", () => {
  assert.deepEqual(tailProblems(releaseWorkflow()), []);
});

test("a tail that moves, is read, may fail quietly or installs another onevcs is refused", () => {
  const cases = {
    "is not publish's last step": (w) => w.jobs.publish.steps.push({ run: "echo after" }),
    "needs publish": (w) => {
      w.jobs.after = { needs: ["publish"], "runs-on": "ubuntu-latest", steps: [{ run: "true" }] };
    },
    "a step reads the tail's outcome": (w) => {
      w.jobs.publish.steps.at(-1).id = "tail";
      w.jobs["build-linux"].steps.push({ if: "steps.tail.outcome == 'failure'", run: "true" });
    },
    "reads publish's result": (w) => {
      w.jobs["build-linux"].if = "needs.publish.result == 'failure'";
    },
    "carries continue-on-error": (w) => {
      w.jobs.publish.steps.at(-1)["continue-on-error"] = true;
    },
    "not handed the job's token": (w) => {
      delete w.jobs.publish.steps.at(-1).env;
    },
    "does not grant actions: read": (w) => {
      delete w.jobs.publish.permissions.actions;
    },
    "does not grant pull-requests: read": (w) => {
      w.jobs.publish.permissions["pull-requests"] = "write";
    },
    "installs the onevcs CLI": (w) => {
      w.jobs.publish.steps.splice(-2, 1);
    },
    "not the adopted": (w) => {
      const install = w.jobs.publish.steps.at(-2);
      install.run = install.run.replace(`onevcs-cli@${ADOPTED_ONEVCS}`, "onevcs-cli@0.56.2");
    },
  };
  for (const [expected, change] of Object.entries(cases)) {
    const workflow = releaseWorkflow();
    change(workflow);
    const found = tailProblems(workflow);
    assert.ok(
      found.some((problem) => problem.includes(expected)),
      `${expected}: ${JSON.stringify(found)}`,
    );
  }
});

// A recording is exactly what `gh api --include` printed: the status line, the headers, each
// ending in CRLF, a blank line and the body.
function response(text) {
  const split = text.indexOf("\r\n\r\n");
  return {
    head: text.slice(0, split + 4),
    status: Number(text.slice(0, text.indexOf("\n")).split(" ")[1]),
    body: JSON.parse(text.slice(split + 4)),
  };
}
const pathOf = (recording) => recording.sidecar.path;

// What the double serves for a recording: the recording itself, byte for byte, but for the one
// change its sidecar names — the job running the tail is still in progress when the tail reads it.
function served(recording) {
  const change = recording.sidecar.served_in_progress;
  if (!change) return { text: recording.text, ...recording.response };
  const answer = structuredClone(recording.response.body);
  const jobs = answer.jobs.filter((job) => job.name === change.job);
  assert.equal(jobs.length, 1, `${recording.name} records one ${change.job} job`);
  Object.assign(jobs[0], { status: "in_progress", conclusion: null, completed_at: null });
  return {
    ...recording.response,
    text: recording.response.head + JSON.stringify(answer),
    body: answer,
  };
}

// One recorded release: its recordings, the paths they answer, and the pull requests and runs
// the tail selects from them.
function loadRelease(release) {
  const { dir, run: releaseRun } = RELEASES[release];
  const fixtures = new URL(`./fixtures/release-tail/${dir}`, import.meta.url);
  const recordings = readdirSync(fixtures)
    .filter((name) => name.endsWith(".http"))
    .map((name) => {
      const text = readFileSync(new URL(name, fixtures), "utf8");
      return {
        name,
        text,
        response: response(text),
        sidecar: JSON.parse(readFileSync(new URL(`${name}.source.json`, fixtures), "utf8")),
      };
    });
  const ok = recordings.filter((r) => r.response.status === 200);
  const byPath = new Map(ok.map((r) => [pathOf(r), r]));
  const body = (path) => byPath.get(path).response.body;
  const publishRecord = ok.find((r) => r.sidecar.served_in_progress);
  const publishJob = publishRecord.response.body.jobs.find((job) => job.name === "publish");
  // The pull requests the release carries: every commit the comparison lists, by the merged pull
  // request the closed page names it the merge commit of.
  const compare = ok.find((r) => pathOf(r).includes("/compare/")).response.body;
  const pulls = ok.find((r) => pathOf(r).includes("/pulls?")).response.body;
  const releaseCommit = compare.commits.at(-1).sha;
  const pullRequests = compare.commits.map((commit) => {
    const pull = pulls.find((p) => p.merge_commit_sha === commit.sha);
    assert.ok(pull, `commit ${commit.sha} maps to a merged pull request`);
    return { number: pull.number, head: pull.head.sha, merged: ts(pull.merged_at) };
  });
  const releaseListing = `repos/${REPOSITORY}/actions/runs?per_page=100&head_sha=${releaseCommit}`;
  const atRelease = () => body(releaseListing).workflow_runs;
  return {
    release,
    releaseRun,
    recordings,
    ok,
    refusal: recordings.find((r) => r.response.status !== 200),
    byPath,
    body,
    publishRecord,
    tailRunner: publishJob.runner_name,
    // The moment the tail's own step started on GitHub, from the step the recording names.
    tailStep: publishJob.steps.find((step) => step.name === "Report the release-tail budgets"),
    pullRequests,
    releaseListing,
    // The runs the CD figures count: push and release runs at the release commit, but the
    // excluded workflow's and a run that concluded skipped, which ran nothing.
    deliveryRuns: () =>
      atRelease().filter(
        (run) =>
          ["push", "release"].includes(run.event) &&
          !EXCLUDED.includes(run.name) &&
          run.conclusion !== "skipped",
      ),
    excludedRuns: () => atRelease().filter((run) => EXCLUDED.includes(run.name)),
    skippedRuns: () =>
      atRelease().filter(
        (run) =>
          ["push", "release"].includes(run.event) &&
          !EXCLUDED.includes(run.name) &&
          run.conclusion === "skipped",
      ),
  };
}
const ts = (stamp) => Date.parse(stamp.replace(/\.\d+Z$/, "Z")) / 1000;
const V12 = loadRelease("v0.12.13");
const V13 = loadRelease("v0.13.0");

// `gh api --include` as onevcs's journeys write it: the response routed for the request asked,
// the request recorded, and a failure the way gh fails on an error status.
const DOUBLE = `#!/bin/sh
dir="$(dirname "$0")/.."
for arg in "$@"; do last="$arg"; done
request="\${last#https://api.github.com/}"
printf '%s\\n' "$request" >> "$dir/served"
n=$(awk -F '\\t' -v r="$request" '$1 == r { print $2; exit }' "$dir/routes.tsv")
if [ -z "$n" ]; then
  echo "replay: no recording for $request" >&2
  exit 1
fi
cat "$dir/responses/$n"
if [ -f "$dir/responses/$n.refusal" ]; then
  cat "$dir/responses/$n.refusal" >&2
  exit 1
fi
`;

const scratch = mkdtempSync(
  join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir(), "release-tail-"),
);
after(() => rmSync(scratch, { recursive: true, force: true }));
let cases = 0;

function replay(serve) {
  const dir = join(scratch, `double-${cases++}`);
  mkdirSync(join(dir, "bin"), { recursive: true });
  mkdirSync(join(dir, "responses"));
  writeFileSync(join(dir, "bin", "gh"), DOUBLE);
  chmodSync(join(dir, "bin", "gh"), 0o755);
  const routes = serve.map((recording, n) => {
    const answer = served(recording);
    writeFileSync(join(dir, "responses", String(n)), answer.text);
    if (answer.status >= 400)
      writeFileSync(
        join(dir, "responses", `${n}.refusal`),
        `gh: ${answer.body.message ?? "error"} (HTTP ${answer.status})\n`,
      );
    return `${pathOf(recording)}\t${n}\n`;
  });
  writeFileSync(join(dir, "routes.tsv"), routes.join(""));
  return {
    program: join(dir, "bin", "gh"),
    served: () => {
      try {
        return readFileSync(join(dir, "served"), "utf8").split("\n").filter(Boolean);
      } catch {
        return [];
      }
    },
  };
}

const toolPath = [];
before(() => {
  const steps = releaseWorkflow().jobs.publish.steps;
  const install = steps.slice(0, -1).find((step) => /\bonevcs-cli@/.test(step.run ?? ""));
  const runnerTemp = join(scratch, "install-runner-temp");
  mkdirSync(runnerTemp);
  const githubPath = join(scratch, "github-path");
  writeFileSync(githubPath, "");
  const ran = spawnSync("bash", ["--noprofile", "--norc", "-eo", "pipefail", "-c", install.run], {
    cwd: root,
    env: { ...process.env, RUNNER_TEMP: runnerTemp, GITHUB_PATH: githubPath },
    encoding: "utf8",
  });
  assert.equal(ran.status, 0, `publish's install step failed: ${ran.stdout}${ran.stderr}`);
  toolPath.push(...readFileSync(githubPath, "utf8").split("\n").filter(Boolean));
  const version = spawnSync("onevcs", ["--version"], {
    env: { PATH: [...toolPath, process.env.PATH].join(":") },
    encoding: "utf8",
  });
  assert.equal(version.stdout.trim(), `onevcs ${ADOPTED_ONEVCS}`);
});

// A checkout as publish's is: the root budgets.yaml (or a test copy of it) and the figures copy.
function checkout(budgets = readFileSync(join(root, "budgets.yaml"), "utf8")) {
  const dir = join(scratch, `checkout-${cases++}`);
  mkdirSync(join(dir, "scripts"), { recursive: true });
  writeFileSync(join(dir, "budgets.yaml"), budgets);
  copyFileSync(
    join(root, "scripts/actions-budget-figures.sh"),
    join(dir, "scripts/actions-budget-figures.sh"),
  );
  return dir;
}

// The last step of publish, run as written in a runner's environment for the release event and
// nothing of this process's own (no GITHUB_* of the CI running these tests leaks in).
function tail({ double, dir = checkout(), runnerTemp, rel = V12 }) {
  const step = releaseWorkflow().jobs.publish.steps.at(-1);
  const event = join(scratch, `event-${cases++}.json`);
  writeFileSync(event, JSON.stringify({ action: "published", release: { tag_name: rel.release } }));
  const home = join(scratch, `home-${cases++}`);
  mkdirSync(home);
  if (!runnerTemp) {
    runnerTemp = join(scratch, `runner-temp-${cases++}`);
    mkdirSync(runnerTemp);
  }
  const started = Math.floor(Date.now() / 1000);
  const ran = spawnSync("bash", ["--noprofile", "--norc", "-eo", "pipefail", "-c", step.run], {
    cwd: dir,
    env: {
      PATH: [...toolPath, process.env.PATH].join(":"),
      HOME: home,
      ONEVCS_HOME: join(home, "onevcs"),
      ONEVCS_GH: double.program,
      GH_TOKEN: step.env.GH_TOKEN === JOB_TOKEN ? "replay-token" : "",
      GITHUB_REPOSITORY: REPOSITORY,
      GITHUB_EVENT_PATH: event,
      GITHUB_RUN_ID: String(rel.releaseRun),
      RUNNER_NAME: rel.tailRunner,
      RUNNER_TEMP: runnerTemp,
    },
    encoding: "utf8",
    timeout: 600_000,
  });
  const ended = Math.ceil(Date.now() / 1000);
  const output = ran.stdout + ran.stderr;
  const results = Object.fromEntries(
    [
      ...output.matchAll(
        /^budget (\S+): (?:actual (\d+) \S+, budget (\d+) \S+, .* — (within|over)|(error) — .*?);/gm,
      ),
    ].map((m) => [
      m[1],
      { value: m[2] && Number(m[2]), threshold: m[3] && Number(m[3]), verdict: m[4] ?? m[5] },
    ]),
  );
  return { status: ran.status, output, results, started, ended, runnerTemp };
}

// A counted job is completed, not skipped and taken by a runner — GitHub records a job no runner
// took with a null or an empty runner_name, which onevcs answers alike — or the job running the
// tail, still in progress, timed to the tail's moment.
const counted = (rel, now) => (job) => {
  if (job.status === "completed" && job.conclusion !== "skipped" && job.runner_name)
    return {
      ...job,
      secs: Math.max(ts(job.completed_at) - ts(job.started_at), 0),
      end: ts(job.completed_at),
    };
  if (
    job.run_id === rel.releaseRun &&
    job.status === "in_progress" &&
    job.runner_name === rel.tailRunner
  )
    return { ...job, secs: Math.max(now - ts(job.started_at), 0), end: now };
  return null;
};
const jobsPath = (runId) =>
  `repos/${REPOSITORY}/actions/runs/${runId}/jobs?per_page=100&filter=latest`;
const jobsOf = (rel, runId) => served(rel.byPath.get(jobsPath(runId))).body.jobs;

function runsAt(rel, listing, served) {
  return rel.body(listing).workflow_runs.filter((run) => served.includes(jobsPath(run.id)));
}
const prListing = (pr) =>
  `repos/${REPOSITORY}/actions/runs?per_page=100&head_sha=${pr.head}&event=pull_request`;
const releaseOwn = (rel) => [
  `repos/${REPOSITORY}/releases?per_page=100`,
  `repos/${REPOSITORY}/compare/`,
  `repos/${REPOSITORY}/pulls?`,
  rel.releaseListing,
];

function expected(rel, served, now) {
  const prs = rel.pullRequests.map((pr) => {
    const runs = runsAt(rel, prListing(pr), served);
    const jobs = runs
      .flatMap((run) => jobsOf(rel, run.id))
      .map(counted(rel, now))
      .filter(Boolean);
    const requests = served.filter(
      (path) =>
        path === prListing(pr) ||
        runs.some((run) => path.startsWith(`repos/${REPOSITORY}/actions/runs/${run.id}/`)),
    );
    return { ...pr, runs, jobs, requests };
  });
  const cdRuns = rel.deliveryRuns();
  const cdJobs = cdRuns
    .flatMap((run) => jobsOf(rel, run.id))
    .map(counted(rel, now))
    .filter(Boolean);
  const own = served.filter(
    (path) =>
      releaseOwn(rel).some((prefix) => path === prefix || path.startsWith(prefix)) ||
      cdRuns.some((run) => path.startsWith(`repos/${REPOSITORY}/actions/runs/${run.id}/`)),
  );
  return {
    "onetaskgraph-ci-check-time": Math.max(...prs.flatMap((pr) => pr.jobs.map((job) => job.secs))),
    "onetaskgraph-cd-job-time": Math.max(...cdJobs.map((job) => job.secs)),
    "onetaskgraph-release-cycle-time": Math.max(
      ...prs.map(
        (pr) =>
          Math.max(...pr.jobs.map((job) => job.end)) -
          Math.min(...pr.runs.map((run) => ts(run.created_at))) +
          (now - pr.merged),
      ),
    ),
    "onetaskgraph-ci-job-count": Math.max(...prs.map((pr) => pr.jobs.length)),
    "onetaskgraph-cd-job-count": cdJobs.length,
    "onetaskgraph-release-read-requests": own.length,
    "onetaskgraph-pr-read-requests": Math.max(...prs.map((pr) => pr.requests.length)),
  };
}
// A figure timed to the tail's own moment lies between the one computed at the step's start and
// the one at its end; every other figure is exact.
function assertFigures(rel, run, served) {
  const low = expected(rel, served, run.started);
  const high = expected(rel, served, run.ended);
  for (const id of TAIL_IDS) {
    const value = run.results[id]?.value;
    assert.ok(
      value >= Math.min(low[id], high[id]) && value <= Math.max(low[id], high[id]),
      `${rel.release} ${id} reported ${value}, computed ${low[id]}..${high[id]}`,
    );
  }
  return low;
}

let recorded;
test("over release v0.12.13 the tail reports all seven budgets with their breakdowns", () => {
  const double = replay(V12.ok);
  const run = tail({ double });
  const servedPaths = double.served();
  recorded = { run, served: servedPaths };
  const budgets = parse(readFileSync(join(root, "budgets.yaml"), "utf8")).budgets;
  assert.deepEqual(Object.keys(run.results).sort(), [...TAIL_IDS].sort(), run.output);
  const figures = assertFigures(V12, run, servedPaths);
  // The step's exit is onebudgetspec's: 1 when any reading is over, 0 when all are within.
  const over = TAIL_IDS.filter((id) => run.results[id].verdict === "over");
  for (const id of TAIL_IDS) {
    const threshold = budgets.find((budget) => budget.id === id).threshold;
    assert.equal(run.results[id].threshold, threshold, id);
    assert.equal(run.results[id].verdict, figures[id] > threshold ? "over" : "within", id);
  }
  assert.equal(run.status, over.length ? 1 : 0, run.output);
  for (const pr of V12.pullRequests)
    assert.match(run.output, new RegExp(`#${pr.number}: checks \\d+s \\+ merge to release \\d+s`));
  assert.match(run.output, /Release \/ publish: \d+s/);
  assert.match(run.output, /longest per runner OS:/);
  assert.match(run.output, /per workflow:/);
  assert.match(run.output, /release-changes --tag v0\.12\.13: 3 requests/);
  assert.match(
    run.output,
    new RegExp(`left out by --exclude-workflow: ${WARMER} \\(run \\d+, push\\)`),
  );
});

test("the CD budgets name the cache warmer's workflow, and only it, with --exclude-workflow", () => {
  const excluding = {
    "onetaskgraph-cd-job-time": EXCLUDED,
    "onetaskgraph-cd-job-count": EXCLUDED,
    "onetaskgraph-release-read-requests": EXCLUDED,
  };
  for (const budget of TAIL_BUDGETS())
    assert.deepEqual(excludedBy(budget.command), excluding[budget.id] ?? [], budget.id);
  assert.equal(WARMER, "rust cache warm");
  assert.ok(V12.excludedRuns().length, "the recorded release commit carries a warmer run");
});

test("each onevcs read is made once across the seven figures", () => {
  const { served } = recorded;
  assert.equal(new Set(served).size, served.length, `a read was made twice: ${served.join("\n")}`);
  const warmerJobs = V12.excludedRuns().map((run) => jobsPath(run.id));
  assert.deepEqual(
    [...served].sort(),
    V12.ok
      .map(pathOf)
      .filter((path) => !warmerJobs.includes(path))
      .sort(),
    "every recording served but the excluded warmer's jobs, and nothing else",
  );
});

test("the read figures are the requests the double served", () => {
  const { run, served } = recorded;
  const figures = expected(V12, served, run.started);
  assert.equal(
    run.results["onetaskgraph-release-read-requests"].value,
    figures["onetaskgraph-release-read-requests"],
  );
  assert.equal(
    run.results["onetaskgraph-pr-read-requests"].value,
    figures["onetaskgraph-pr-read-requests"],
  );
  // Every request falls to the release or to one pull request, so a read neither figure
  // counts cannot hide between them.
  assert.equal(
    figures["onetaskgraph-release-read-requests"] +
      V12.pullRequests
        .map(
          (pr) =>
            served.filter(
              (path) =>
                path === prListing(pr) ||
                runsAt(V12, prListing(pr), served).some((r) => path.includes(`/runs/${r.id}/`)),
            ).length,
        )
        .reduce((a, b) => a + b, 0),
    served.length,
  );
});

test("the selection is the release's: the pull requests' heads and the release commit's CD runs", () => {
  const { served } = recorded;
  for (const pr of V12.pullRequests)
    assert.ok(served.includes(prListing(pr)), `#${pr.number}'s head was read`);
  for (const run of V12.deliveryRuns())
    assert.ok(
      runsAt(V12, V12.releaseListing, served).some((r) => r.id === run.id),
      run.name,
    );
  // The cache warmer's run at the release commit is excluded, and its jobs are never read.
  for (const run of V12.excludedRuns())
    assert.ok(!served.some((path) => path.includes(`/runs/${run.id}/`)), run.name);
  // A scheduled run at the release commit is no CD run, and its jobs are never read.
  for (const run of V12.body(V12.releaseListing).workflow_runs.filter(
    (r) => r.event === "schedule",
  ))
    assert.ok(!served.some((path) => path.includes(`/runs/${run.id}/`)), run.name);
});

test("a threshold below a recorded figure turns the step's exit non-zero naming that budget", () => {
  const source = parse(readFileSync(join(root, "budgets.yaml"), "utf8"));
  const runnerTemp = recorded.run.runnerTemp;
  const copy = (lowered) => {
    const document = structuredClone(source);
    for (const budget of document.budgets)
      if (TAIL_IDS.includes(budget.id))
        budget.threshold = budget.id in lowered ? lowered[budget.id] : 10 ** 9;
    return stringify(document);
  };
  const within = tail({ double: replay(V12.ok), dir: checkout(copy({})), runnerTemp });
  assert.equal(within.status, 0, within.output);
  const figures = expected(V12, recorded.served, recorded.run.started);
  for (const id of TAIL_IDS) {
    const run = tail({
      double: replay(V12.ok),
      dir: checkout(copy({ [id]: figures[id] - 1 })),
      runnerTemp,
    });
    assert.equal(run.status, 1, `${id}: ${run.output}`);
    assert.equal(run.results[id].verdict, "over", `${id}: ${run.output}`);
    for (const other of TAIL_IDS.filter((o) => o !== id))
      assert.equal(run.results[other].verdict, "within", other);
  }
});

test("an error status from GitHub fails the step and reports no figure", () => {
  const { refusal } = V12;
  const serve = [refusal, ...V12.ok.filter((r) => pathOf(r) !== pathOf(refusal))];
  const run = tail({ double: replay(serve) });
  assert.notEqual(run.status, 0, run.output);
  assert.equal(run.status, 3, run.output);
  for (const id of TAIL_IDS)
    assert.equal(run.results[id]?.verdict, "error", `${id}: ${run.output}`);
  assert.match(run.output, /Bad credentials/);
});

test("every recording names its API path and where it was fetched from", () => {
  assert.ok(V12.refusal, "an error status is recorded");
  for (const rel of [V12, V13])
    for (const recording of rel.recordings) {
      const { path, repository, fetched_from, recorded } = recording.sidecar;
      assert.ok(path.startsWith(`repos/${REPOSITORY}/`), recording.name);
      assert.equal(repository, REPOSITORY, recording.name);
      assert.equal(fetched_from.release, rel.release, recording.name);
      assert.ok(
        recorded.read_only && recorded.with.startsWith(`gh api --include --method GET '${path}'`),
        recording.name,
      );
      assert.match(recording.response.head, /^HTTP\/2\.0 \d{3} /, recording.name);
      assert.ok(Object.keys(recording.response.body).length, recording.name);
    }
  for (const rel of [V12, V13])
    assert.deepEqual(
      rel.recordings.filter((r) => r.sidecar.served_in_progress).map((r) => r.name),
      [rel.publishRecord.name],
      `${rel.release}: one recording carries the one change`,
    );
});

// v0.13.0's release commit carries four push or release runs: release-plz's, the Release run, the
// main-push cache warmer's and a "Visual docs" run that concluded skipped, every job of it skipped
// by its condition. The double serves the jobs of all four, so a script that reads the warmer's
// or the skipped run's jobs is caught counting the read, not failing on a missing recording. That
// skipped run's jobs read was v0.13.0's seventh release read, against a budget of six.
const CD_WORKFLOWS = ["Release", "Release-plz"];
test("over release v0.13.0 the CD figures count release-plz and Release, and neither the warmer nor the skipped run", () => {
  const double = replay(V13.ok);
  const run = tail({ double, rel: V13 });
  const served = double.served();
  assert.deepEqual(Object.keys(run.results).sort(), [...TAIL_IDS].sort(), run.output);
  assertFigures(V13, run, served);
  assert.equal(new Set(served).size, served.length, `a read was made twice: ${served.join("\n")}`);

  // release-plz and the Release workflow are the runs counted, and both have their jobs read.
  assert.deepEqual(
    V13.deliveryRuns()
      .map((r) => r.name)
      .sort(),
    CD_WORKFLOWS,
  );
  for (const cd of V13.deliveryRuns()) assert.ok(served.includes(jobsPath(cd.id)), cd.name);
  const cdJobs = V13.deliveryRuns().flatMap((cd) =>
    jobsOf(V13, cd.id).map(counted(V13, run.started)).filter(Boolean),
  );
  assert.deepEqual(
    [...new Set(cdJobs.map((job) => job.workflow_name))].sort(),
    CD_WORKFLOWS,
    "every counted CD job is release-plz's or the Release workflow's",
  );
  assert.equal(run.results["onetaskgraph-cd-job-count"].value, 6, run.output);

  // The warmer counts toward no CD figure: its jobs are never read, and leaving it out is said.
  const [warmer] = V13.excludedRuns().filter((r) => r.event === "push");
  assert.ok(warmer, "v0.13.0's release commit carries a warmer push run");
  assert.ok(!served.includes(jobsPath(warmer.id)), "the warmer's jobs were read");
  assert.match(
    run.output,
    new RegExp(`left out by --exclude-workflow: ${WARMER} \\(run ${warmer.id}, push\\)`),
  );
  const warmerJobs = jobsOf(V13, warmer.id).map(counted(V13, run.started)).filter(Boolean);
  assert.ok(warmerJobs.length, "the warmer ran jobs a figure could have counted");
  // The longest CD job is a counted run's, whatever the warmer's jobs took.
  assert.equal(
    run.results["onetaskgraph-cd-job-time"].value,
    Math.max(...cdJobs.map((job) => job.secs)),
    run.output,
  );
  assert.equal(run.results["onetaskgraph-cd-job-count"].value, cdJobs.length, run.output);

  // The skipped run costs no jobs request, and leaving it out is said.
  const skipped = V13.skippedRuns();
  assert.deepEqual(
    skipped.map((r) => [r.name, r.event]),
    [["Visual docs", "push"]],
  );
  for (const r of skipped) {
    assert.ok(!served.includes(jobsPath(r.id)), `${r.name}'s jobs were read`);
    assert.match(
      run.output,
      new RegExp(`left out as concluded skipped: ${r.name} \\(run ${r.id}, push\\)`),
    );
  }

  // Every read is the release's own four or one of the counted runs' jobs, so the release read
  // figure is six, within the threshold budgets.yaml states.
  assert.deepEqual(
    [...served].sort(),
    V13.ok
      .map(pathOf)
      .filter((path) => ![warmer, ...skipped].some((r) => path === jobsPath(r.id)))
      .sort(),
  );
  const threshold = TAIL_BUDGETS().find(
    (b) => b.id === "onetaskgraph-release-read-requests",
  ).threshold;
  const reads = run.results["onetaskgraph-release-read-requests"];
  assert.equal(reads.value, 6, run.output);
  assert.equal(reads.threshold, threshold);
  assert.ok(reads.value <= threshold, `${reads.value} release reads over ${threshold}`);
  assert.equal(reads.verdict, "within", run.output);
  assert.match(run.output, /release-changes --tag v0\.13\.0: 3 requests/);
});

// Timed to the moment the tail's own step started on GitHub — `--now`, which every figure timed
// to the tail's moment takes — the budgets' own commands report v0.13.0 as that release would
// have read with this script: every figure within its threshold.
test("at the moment v0.13.0's tail ran, every release-tail figure is within its budget", (t) => {
  const now = V13.tailStep.started_at;
  const document = parse(readFileSync(join(root, "budgets.yaml"), "utf8"));
  for (const budget of document.budgets)
    if (
      TAIL_IDS.includes(budget.id) &&
      ["longest-job", "job-count", "release-cycle-time"].includes(budget.command[2])
    )
      budget.command = [...budget.command, "--now", now];
  const double = replay(V13.ok);
  const run = tail({ double, rel: V13, dir: checkout(stringify(document)) });
  const figures = expected(V13, double.served(), ts(now));
  for (const id of TAIL_IDS) {
    const threshold = document.budgets.find((b) => b.id === id).threshold;
    assert.equal(run.results[id]?.value, figures[id], `${id}: ${run.output}`);
    assert.equal(run.results[id].threshold, threshold, id);
    assert.equal(run.results[id].verdict, "within", `${id}: ${run.output}`);
    t.diagnostic(`${id}: ${figures[id]} against ${threshold}`);
  }
  assert.equal(run.status, 0, run.output);
});

// The copy is dero-skills' create-repo source, edited only there. create-repo's
// check_repo_baseline.py compares it with the source as it then stands; this refuses the one
// drift a copy can make on its own — an edit in place — by its own header, whose `sha256` line
// is the digest of every byte after it.
test("the figures script is the unedited create-repo copy its header names", () => {
  const text = readFileSync(join(root, "scripts/actions-budget-figures.sh"), "utf8");
  const lines = text.split("\n");
  assert.equal(
    lines[2],
    "# source: github.com/nickderobertis/dero-skills/skills/bootstrap/create-repo/assets/budgets/actions-budget-figures.sh",
  );
  assert.equal(lines[3], "# version: 10");
  const digest = lines[4].match(/^# sha256: ([0-9a-f]{64})$/)?.[1];
  const body = lines.slice(5).join("\n");
  assert.equal(
    createHash("sha256").update(body).digest("hex"),
    digest,
    "the copy was edited in place",
  );
  assert.equal(digest, "13ac7d79aaa3902dc7cfe87449283a05ba397fbc0612701e9b788ed59daf438d");
});

// The reconciliation script, against a stand-in for gh that answers each path with its
// recording, or with what GitHub would answer later: rewritten headers and timestamps alone,
// a changed body, or no answer at all.
test("the reconciliation names a recording GitHub no longer answers, and ignores what GitHub rewrites", () => {
  const dir = join(scratch, `reconcile-${cases++}`);
  mkdirSync(dir);
  const answer = join(dir, "answer.mjs");
  writeFileSync(
    answer,
    `import { readdirSync, readFileSync } from "node:fs";
const dir = ${JSON.stringify(fileURLToPath(new URL("./fixtures/release-tail/recordings/", import.meta.url)))};
const path = process.argv.at(-1);
// GitHub answers the releases path 401 under an invalid token, as the reconciliation asks it.
const invalid = (process.env.GH_TOKEN ?? "").startsWith("ghp_invalid");
const name = readdirSync(dir).find((n) => {
  if (!n.endsWith(".http.source.json")) return false;
  const sidecar = JSON.parse(readFileSync(dir + "/" + n, "utf8"));
  return sidecar.path === path && sidecar.recorded.with.endsWith("under an invalid token") === invalid;
});
if (!name || process.env.ANSWER === "none") process.exit(1);
let text = readFileSync(dir + "/" + name.replace(/\\.source\\.json$/, ""), "utf8");
if (process.env.ANSWER === "rewritten")
  text = text
    .replace(/^Date: .*$/m, "Date: Sun, 01 Nov 2026 00:00:00 GMT")
    .replace(/^X-Ratelimit-Remaining: .*$/im, "X-Ratelimit-Remaining: 1")
    .replace(/"updated_at":"[^"]*"/g, '"updated_at":"2026-11-01T00:00:00Z"');
if (process.env.ANSWER === "changed" && name.startsWith("runs-at-pr-3824-head"))
  text = text.replace('"total_count":', '"total_count":1');
// One recording drifting in a way the reconciliation must see: its status, a header it keeps,
// or a field changing from a list to an object.
if (name.startsWith("runs-at-pr-3824-head")) {
  if (process.env.ANSWER === "status") text = text.replace(/^HTTP\\/2\\.0 200 OK/, "HTTP/2.0 404 Not Found");
  if (process.env.ANSWER === "header") text = text.replace(/^Content-Type: .*$/im, "Content-Type: text/plain");
  if (process.env.ANSWER === "container") {
    const split = text.indexOf("\\r\\n\\r\\n") + 4;
    const body = JSON.parse(text.slice(split));
    body.workflow_runs = {};
    text = text.slice(0, split) + JSON.stringify(body);
  }
}
if (process.env.ANSWER === "no-separator") text = text.replaceAll("\\r\\n", "\\n");
if (process.env.ANSWER === "no-status") text = text.replace(/^HTTP\\/[^\\n]*/, "garbage");
if (process.env.ANSWER === "no-json") text = text.slice(0, text.indexOf("\\r\\n\\r\\n") + 5);
process.stdout.write(text);
`,
  );
  mkdirSync(join(dir, "bin"));
  writeFileSync(join(dir, "bin", "gh"), `#!/bin/sh\nexec "${process.execPath}" "${answer}" "$@"\n`);
  chmodSync(join(dir, "bin", "gh"), 0o755);
  const reconcile = (ANSWER) =>
    spawnSync(
      process.execPath,
      [fileURLToPath(new URL("./fixtures/release-tail/reconcile.mjs", import.meta.url))],
      {
        env: { ...process.env, PATH: `${join(dir, "bin")}:${process.env.PATH}`, ANSWER },
        encoding: "utf8",
      },
    );
  const same = reconcile("same");
  assert.equal(same.status, 0, same.stderr);
  const rewritten = reconcile("rewritten");
  assert.equal(rewritten.status, 0, rewritten.stderr);
  const changed = reconcile("changed");
  assert.equal(changed.status, 1, changed.stderr);
  assert.match(
    changed.stderr,
    /^runs-at-pr-3824-head\.http: GitHub now answers otherwise at body\.total_count$/m,
  );
  assert.match(changed.stderr, /1 recording\(s\) no longer match GitHub/);
  for (const [answer, at] of [
    ["status", "status 200 is now 404"],
    ["header", "header content-type"],
    ["container", "body.workflow_runs"],
  ]) {
    const drifted = reconcile(answer);
    assert.equal(drifted.status, 1, `${answer}: ${drifted.stderr}`);
    assert.match(
      drifted.stderr,
      new RegExp(`^runs-at-pr-3824-head\\.http: GitHub now answers otherwise at .*${at}`, "m"),
      answer,
    );
  }
  for (const [answer, says] of [
    ["none", /holds no response/],
    ["no-separator", /holds no response/],
    ["no-status", /starts with no HTTP status line: garbage/],
    ["no-json", /JSON/],
  ]) {
    const refused = reconcile(answer);
    assert.equal(refused.status, 2, `${answer}: ${refused.stderr}`);
    assert.match(refused.stderr, says, answer);
    assert.match(refused.stderr, /next: sign gh in/, answer);
  }
});
