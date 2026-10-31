import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
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
// serving the responses recorded, read-only, from this repository's own release v0.12.13 and
// the runs that carried it (fixtures/release-tail/). Every figure is held to the one this test
// computes from those recordings, and every read count to what the double itself served.

const root = fileURLToPath(new URL("../../", import.meta.url));
const FIXTURES = new URL("./fixtures/release-tail/recordings/", import.meta.url);
// The onevcs release this repository adopts: the first carrying `onevcs host` that this plan's
// onevcs-host-queries node released, on npm.
const ADOPTED_ONEVCS = "0.57.4";
const REPOSITORY = "nickderobertis/onetaskgraph";
const RELEASE = "v0.12.13";
const RELEASE_RUN = 41026720026;
// The job's own token, as a step's env names it.
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

const releaseWorkflow = () =>
  parse(readFileSync(join(root, ".github/workflows/release.yml"), "utf8"));
const rootPackage = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));

// What is wrong with `publish`'s tail, if anything.
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

// ---- The recordings, and the double that serves them ------------------------------------------

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
const recordings = readdirSync(FIXTURES)
  .filter((name) => name.endsWith(".http"))
  .map((name) => {
    const text = readFileSync(new URL(name, FIXTURES), "utf8");
    return {
      name,
      text,
      response: response(text),
      sidecar: JSON.parse(readFileSync(new URL(`${name}.source.json`, FIXTURES), "utf8")),
    };
  });
const pathOf = (recording) => recording.sidecar.path;
const ok = recordings.filter((r) => r.response.status === 200);
const refusal = recordings.find((r) => r.response.status !== 200);
const byPath = new Map(ok.map((r) => [pathOf(r), r]));
const body = (path) => byPath.get(path).response.body;

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
const publishRecord = ok.find((r) => r.sidecar.served_in_progress);
const TAIL_RUNNER = publishRecord.response.body.jobs.find(
  (job) => job.name === "publish",
).runner_name;

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

// ---- publish's own steps, run as written -------------------------------------------------------

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
function tail({ double, dir = checkout(), runnerTemp }) {
  const step = releaseWorkflow().jobs.publish.steps.at(-1);
  const event = join(scratch, "event.json");
  writeFileSync(event, JSON.stringify({ action: "published", release: { tag_name: RELEASE } }));
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
      GITHUB_RUN_ID: String(RELEASE_RUN),
      RUNNER_NAME: TAIL_RUNNER,
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

// ---- The figures, computed from the recordings -------------------------------------------------

const ts = (stamp) => Date.parse(stamp.replace(/\.\d+Z$/, "Z")) / 1000;
// A counted job is completed, not skipped and taken by a runner — GitHub records a job no runner
// took with a null or an empty runner_name, which onevcs answers alike — or the job running the
// tail, still in progress, timed to the tail's moment.
const counted = (now) => (job) => {
  if (job.status === "completed" && job.conclusion !== "skipped" && job.runner_name)
    return {
      ...job,
      secs: Math.max(ts(job.completed_at) - ts(job.started_at), 0),
      end: ts(job.completed_at),
    };
  if (job.run_id === RELEASE_RUN && job.status === "in_progress" && job.runner_name === TAIL_RUNNER)
    return { ...job, secs: Math.max(now - ts(job.started_at), 0), end: now };
  return null;
};
const jobsOf = (runId) =>
  served(byPath.get(`repos/${REPOSITORY}/actions/runs/${runId}/jobs?per_page=100&filter=latest`))
    .body.jobs;

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

// Which runs the double was asked the jobs of, at a head.
function runsAt(listing, served) {
  return body(listing).workflow_runs.filter((run) =>
    served.includes(`repos/${REPOSITORY}/actions/runs/${run.id}/jobs?per_page=100&filter=latest`),
  );
}
const prListing = (pr) =>
  `repos/${REPOSITORY}/actions/runs?per_page=100&head_sha=${pr.head}&event=pull_request`;
const releaseListing = `repos/${REPOSITORY}/actions/runs?per_page=100&head_sha=${releaseCommit}`;
const RELEASE_OWN = [
  `repos/${REPOSITORY}/releases?per_page=100`,
  `repos/${REPOSITORY}/compare/`,
  `repos/${REPOSITORY}/pulls?`,
  releaseListing,
];

function expected(served, now) {
  const prs = pullRequests.map((pr) => {
    const runs = runsAt(prListing(pr), served);
    const jobs = runs
      .flatMap((run) => jobsOf(run.id))
      .map(counted(now))
      .filter(Boolean);
    const requests = served.filter(
      (path) =>
        path === prListing(pr) ||
        runs.some((run) => path.startsWith(`repos/${REPOSITORY}/actions/runs/${run.id}/`)),
    );
    return { ...pr, runs, jobs, requests };
  });
  const cdRuns = body(releaseListing).workflow_runs.filter((run) =>
    ["push", "release"].includes(run.event),
  );
  const cdJobs = cdRuns
    .flatMap((run) => jobsOf(run.id))
    .map(counted(now))
    .filter(Boolean);
  const releaseOwn = served.filter(
    (path) =>
      RELEASE_OWN.some((prefix) => path === prefix || path.startsWith(prefix)) ||
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
    "onetaskgraph-release-read-requests": releaseOwn.length,
    "onetaskgraph-pr-read-requests": Math.max(...prs.map((pr) => pr.requests.length)),
  };
}
// A figure timed to the tail's own moment lies between the one computed at the step's start and
// the one at its end; every other figure is exact.
function assertFigures(run, served) {
  const low = expected(served, run.started);
  const high = expected(served, run.ended);
  for (const id of TAIL_IDS) {
    const value = run.results[id]?.value;
    assert.ok(
      value >= Math.min(low[id], high[id]) && value <= Math.max(low[id], high[id]),
      `${id} reported ${value}, computed ${low[id]}..${high[id]}`,
    );
  }
  return low;
}

// ---- The journeys ------------------------------------------------------------------------------

let recorded;
test("over a recorded release the tail reports all seven budgets with their breakdowns", () => {
  const double = replay(ok);
  const run = tail({ double });
  const servedPaths = double.served();
  recorded = { run, served: servedPaths };
  const budgets = parse(readFileSync(join(root, "budgets.yaml"), "utf8")).budgets;
  assert.deepEqual(Object.keys(run.results).sort(), [...TAIL_IDS].sort(), run.output);
  const figures = assertFigures(run, servedPaths);
  // The step's exit is onebudgetspec's: 1 when any reading is over, 0 when all are within.
  const over = TAIL_IDS.filter((id) => run.results[id].verdict === "over");
  for (const id of TAIL_IDS) {
    const threshold = budgets.find((budget) => budget.id === id).threshold;
    assert.equal(run.results[id].threshold, threshold, id);
    assert.equal(run.results[id].verdict, figures[id] > threshold ? "over" : "within", id);
  }
  assert.equal(run.status, over.length ? 1 : 0, run.output);
  // Each figure's breakdown is in the log: the pull requests, the jobs and the requests.
  for (const pr of pullRequests)
    assert.match(run.output, new RegExp(`#${pr.number}: checks \\d+s \\+ merge to release \\d+s`));
  assert.match(run.output, /Release \/ publish: \d+s/);
  assert.match(run.output, /longest per runner OS:/);
  assert.match(run.output, /per workflow:/);
  assert.match(run.output, /release-changes --tag v0\.12\.13: 3 requests/);
});

test("each onevcs read is made once across the seven figures", () => {
  const { served } = recorded;
  assert.equal(new Set(served).size, served.length, `a read was made twice: ${served.join("\n")}`);
  assert.deepEqual(
    [...served].sort(),
    ok.map(pathOf).sort(),
    "every recording served, and nothing else",
  );
});

test("the read figures are the requests the double served", () => {
  const { run, served } = recorded;
  const figures = expected(served, run.started);
  assert.equal(
    run.results["onetaskgraph-release-read-requests"].value,
    figures["onetaskgraph-release-read-requests"],
  );
  assert.equal(
    run.results["onetaskgraph-pr-read-requests"].value,
    figures["onetaskgraph-pr-read-requests"],
  );
  // And the two together are every request the tail made.
  assert.equal(
    figures["onetaskgraph-release-read-requests"] +
      pullRequests
        .map(
          (pr) =>
            served.filter(
              (path) =>
                path === prListing(pr) ||
                runsAt(prListing(pr), served).some((r) => path.includes(`/runs/${r.id}/`)),
            ).length,
        )
        .reduce((a, b) => a + b, 0),
    served.length,
  );
});

test("the selection is the release's: the pull requests' heads and the release commit's CD runs", () => {
  const { served } = recorded;
  for (const pr of pullRequests)
    assert.ok(served.includes(prListing(pr)), `#${pr.number}'s head was read`);
  const cd = body(releaseListing).workflow_runs.filter((run) =>
    ["push", "release"].includes(run.event),
  );
  for (const run of cd)
    assert.ok(
      runsAt(releaseListing, served).some((r) => r.id === run.id),
      run.name,
    );
  // A scheduled run at the release commit is no CD run, and its jobs are never read.
  for (const run of body(releaseListing).workflow_runs.filter((r) => r.event === "schedule"))
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
  const within = tail({ double: replay(ok), dir: checkout(copy({})), runnerTemp });
  assert.equal(within.status, 0, within.output);
  const figures = expected(recorded.served, recorded.run.started);
  for (const id of TAIL_IDS) {
    const run = tail({
      double: replay(ok),
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
  const serve = [refusal, ...ok.filter((r) => pathOf(r) !== pathOf(refusal))];
  const run = tail({ double: replay(serve) });
  assert.notEqual(run.status, 0, run.output);
  assert.equal(run.status, 3, run.output);
  for (const id of TAIL_IDS)
    assert.equal(run.results[id]?.verdict, "error", `${id}: ${run.output}`);
  assert.match(run.output, /Bad credentials/);
});

test("every recording names its API path and where it was fetched from", () => {
  assert.ok(refusal, "an error status is recorded");
  for (const recording of recordings) {
    const { path, repository, fetched_from, recorded } = recording.sidecar;
    assert.ok(path.startsWith(`repos/${REPOSITORY}/`), recording.name);
    assert.equal(repository, REPOSITORY, recording.name);
    assert.equal(fetched_from.release, RELEASE, recording.name);
    assert.ok(
      recorded.read_only && recorded.with.startsWith(`gh api --include --method GET '${path}'`),
      recording.name,
    );
    assert.match(recording.response.head, /^HTTP\/2\.0 \d{3} /, recording.name);
    assert.ok(Object.keys(recording.response.body).length, recording.name);
  }
  assert.deepEqual(
    recordings.filter((r) => r.sidecar.served_in_progress).map((r) => r.name),
    [publishRecord.name],
    "one recording carries the one change",
  );
});
