import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import { test } from "node:test";
import { read, workflows } from "../workflow-graph.mjs";

// No lock serialises the live suites: every session works only on its own run's board copy
// and leased Linear slot set, so runs execute them side by side. A job-level or
// workflow-level concurrency group on anything that runs a live suite would bring back the
// queue a pull request once waited hours in, so this refuses one wherever it appears.
const LIVE_CREDENTIALS = ["LINEAR_API_KEY", "GH_PROJECTS_TOKEN"];

/** Whether `env` names a live credential. */
const holdsCredential = (env) => LIVE_CREDENTIALS.some((key) => env?.[key] !== undefined);

/** One line that is a `cargo run` of the live janitor and nothing else: no shell operator. */
const JANITOR_LINE =
  /^cargo run(\s+--?[\w-]+)*\s+-p\s+onetaskgraph-live-janitor(\s+--(\s+[^\s;&|`$()<>]+)*)?$/;

/**
 * Whether `run` is nothing but invocations of the live janitor, which removes artifacts and
 * opens no session: every line that is not blank or a comment is exactly one such invocation.
 */
const onlyJanitor = (run) =>
  run
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "" && !line.startsWith("#"))
    .every((line) => JANITOR_LINE.test(line));

/**
 * The jobs of `definitions` that run a live suite: a job with a step that runs any command —
 * `just`, `cargo`, a script or anything else — while a live credential reaches it through the
 * workflow's `env`, the job's or the step's own. A step that invokes only the live janitor is
 * not one: it removes artifacts and opens no session, so its own group serialises no suite.
 */
export function liveSuiteJobs(definitions) {
  const found = [];
  for (const [file, workflow] of Object.entries(definitions)) {
    for (const [name, job] of Object.entries(workflow.jobs ?? {})) {
      const inherited = holdsCredential(workflow.env) || holdsCredential(job.env);
      const runsSuite = (job.steps ?? []).some(
        (step) =>
          typeof step.run === "string" &&
          !onlyJanitor(step.run) &&
          (inherited || holdsCredential(step.env)),
      );
      if (runsSuite) found.push({ file, name, workflow, job });
    }
  }
  return found;
}

/** Every serialisation of a live suite `definitions` declares, as one line each. */
export function liveSerialisations(definitions, sources) {
  const problems = [];
  for (const { file, name, workflow, job } of liveSuiteJobs(definitions)) {
    if (job.concurrency !== undefined) {
      problems.push(`${file} job ${name} runs a live suite under a concurrency group`);
    }
    if (workflow.concurrency !== undefined) {
      problems.push(`${file} runs a live suite (job ${name}) under a workflow concurrency group`);
    }
  }
  for (const [file, text] of Object.entries(sources)) {
    if (text.includes("linear-live-documents")) {
      problems.push(`${file} names the retired linear-live-documents lock`);
    }
  }
  return problems;
}

const workflowSources = () =>
  Object.fromEntries(
    Object.keys(workflows()).map((file) => [file, read(`.github/workflows/${file}`)]),
  );

test("no workflow serialises a live suite", () => {
  const definitions = workflows();
  const jobs = liveSuiteJobs(definitions).map(({ file, name }) => `${file}:${name}`);
  // The scan has to see every lane that opens a live session, or a pass proves nothing.
  for (const expected of ["ci.yml:check", "ci.yml:test-linux", "linear-documents.yml:documents"]) {
    assert.ok(jobs.includes(expected), `${expected} runs a live suite and was not found`);
  }
  assert.deepEqual(liveSerialisations(definitions, workflowSources()), []);
});

test("a live suite is found however its credential and its command reach it", () => {
  const definitions = {
    "direct.yml": {
      env: { GH_PROJECTS_TOKEN: "x" },
      concurrency: { group: "shared" },
      jobs: {
        suite: { steps: [{ run: "cargo test -p onetaskgraph-github-projects --test live" }] },
      },
    },
    "script.yml": {
      jobs: {
        suite: {
          env: { LINEAR_API_KEY: "x" },
          concurrency: { group: "shared" },
          steps: [{ run: "bash scripts/some-live-entry.sh" }],
        },
      },
    },
    "janitor.yml": {
      concurrency: { group: "janitor" },
      jobs: {
        clean: {
          steps: [
            {
              env: { GH_PROJECTS_TOKEN: "x" },
              run: "cargo run --locked -p onetaskgraph-live-janitor -- --cancelled-run",
            },
          ],
        },
      },
    },
    "compound.yml": {
      jobs: {
        suite: {
          concurrency: { group: "shared" },
          steps: [
            {
              env: { LINEAR_API_KEY: "x" },
              run: "cargo run --locked -p onetaskgraph-live-janitor -- --linear; just check",
            },
          ],
        },
      },
    },
    "uncredentialed.yml": {
      concurrency: { group: "shared" },
      jobs: { suite: { steps: [{ run: "just check" }] } },
    },
  };
  assert.deepEqual(
    liveSuiteJobs(definitions).map(({ file, name }) => `${file}:${name}`),
    ["direct.yml:suite", "script.yml:suite", "compound.yml:suite"],
  );
  assert.deepEqual(liveSerialisations(definitions, {}), [
    "direct.yml runs a live suite (job suite) under a workflow concurrency group",
    "script.yml job suite runs a live suite under a concurrency group",
    "compound.yml job suite runs a live suite under a concurrency group",
  ]);
});

test("a reintroduced live lock is refused, at job and at workflow level", () => {
  const definitions = structuredClone(workflows());
  definitions["ci.yml"].jobs["test-linux"].concurrency = { group: "any-shared-group" };
  definitions["linear-documents.yml"].concurrency = { group: "documents" };
  const sources = { "ci.yml": "concurrency:\n  group: linear-live-documents\n" };
  assert.deepEqual(liveSerialisations(definitions, sources), [
    "ci.yml job test-linux runs a live suite under a concurrency group",
    "linear-documents.yml runs a live suite (job documents) under a workflow concurrency group",
    "ci.yml names the retired linear-live-documents lock",
  ]);
});

test("only the daily schedule creates fresh documents and manual dispatch initializes", () => {
  const definitions = workflows();
  const workflow = definitions["linear-documents.yml"];
  assert.deepEqual(Object.keys(workflow.on), ["schedule", "workflow_dispatch"]);
  assert.deepEqual(workflow.on.schedule, [{ cron: "43 6 * * *" }]);
  assert.equal(workflow.on.workflow_dispatch, null);
  const job = workflow.jobs.documents;
  assert.equal(
    job.env.ONETASKGRAPH_LINEAR_DOCUMENT_MODE,
    "$" + "{{ github.event_name == 'schedule' && 'fresh' || 'initialize' }}",
  );
  assert.equal(job.env.ONETASKGRAPH_LIVE_REQUIRED, "1");
  assert.equal(job.env.LINEAR_WRITE_TEAM, "$" + "{{ vars.LINEAR_WRITE_TEAM }}");
  const commands = job.steps.filter((step) => step.run).map((step) => step.run);
  assert.deepEqual(commands, ["just linear-document-creation"]);
  assert.ok(job.steps.some((step) => step.uses === "extractions/setup-just@v4"));
  assert.ok(
    job.steps.some(
      (step) =>
        step.uses === "oven-sh/setup-bun@v2" && step.with["bun-version-file"] === "package.json",
    ),
  );
  assert.match(
    read("justfile"),
    /linear-document-creation:\n {4}@\{\{nx\}\} run-many -t live-documents --projects onetaskgraph-linear,onetaskgraph-linear-e2e --parallel=1/,
  );
  const commandsByLane = {
    "onetaskgraph-linear":
      "cargo test --locked -p onetaskgraph-linear --test live -- real_linear_applies_every_declared_capability_and_leaves_no_residue --exact --nocapture",
    "onetaskgraph-linear-e2e":
      "cargo test --locked -p onetaskgraph-linear-e2e --test e2e -- linear_assets::live_disposable_issue_and_document_assets_copy_and_recopy --exact --nocapture",
  };
  for (const [lane, command] of Object.entries(commandsByLane)) {
    const project = JSON.parse(read(`crates/${lane}/project.json`));
    const target = project.targets["live-documents"];
    assert.equal(target.options.command, command);
    assert.equal(target.cache, false);
    assert.equal(target.parallelism, false);
    assert.ok(!project.targets.check.dependsOn.includes("live-documents"));
    if (lane.endsWith("-e2e")) assert.deepEqual(target.dependsOn, ["onetaskgraph:build"]);
  }
  // The cancellation steps that hand back a run's leases hold the key too, and open no session;
  // scripts/check-live-janitor.sh holds them to their shape.
  const releasesLeases = (step) =>
    step.run ===
    "cargo run --locked -p onetaskgraph-live-janitor -- --linear --release-leases target/linear-leases";
  const credentialed = (job) =>
    definitions["ci.yml"].jobs[job].steps.filter(
      (step) => step.env?.LINEAR_API_KEY && !releasesLeases(step),
    );
  const ordinarySteps = [...credentialed("check"), ...credentialed("test-linux")];
  assert.ok(credentialed("test-linux").length > 0, "test-linux carries a pull request's session");
  for (const step of ordinarySteps) {
    assert.equal(step.env.ONETASKGRAPH_LINEAR_DOCUMENT_MODE, "ordinary");
    assert.match(step.run, /^just ((check|gate)-generated|ci-test-linux)$/);
  }
});

// The operator's two setup recipes reach the lanes only through the variable each lane reads
// and the entry point that reads it, so their spellings are held to the lanes' own.
test("the pool and template recipes set the variables their lanes read", () => {
  const constant = (path, name) =>
    read(path).match(new RegExp(`pub const ${name}: &str = "([^"]+)";`))[1];
  const pool = constant(
    "crates/onetaskgraph-linear/tests/document_lifecycle/mod.rs",
    "POOL_VARIABLE",
  );
  const template = constant(
    "crates/onetaskgraph-github-projects/tests/journey/run_board.rs",
    "TEMPLATE_VARIABLE",
  );
  const recipe = (name) => read("justfile").split(`\n${name} action:\n`)[1].split("\n")[0];
  const poolRecipe = recipe("linear-document-pool");
  // Each recipe hands its lane only an action that lane reads, refused by `just` before any
  // shell sees it, so an operator's argument is never shell syntax.
  assert.ok(
    poolRecipe.includes(
      `${pool}={{ if action =~ '^(dry-run|initialize)$' { action } else { error(`,
    ),
    poolRecipe,
  );
  assert.ok(
    poolRecipe.includes("-t live-documents --projects onetaskgraph-linear,onetaskgraph-linear-e2e"),
    poolRecipe,
  );
  // scripts/nx.sh keeps a successful run's output to itself unless it is handed an
  // --outputStyle, and the operator reads the setup's report off that output.
  assert.ok(poolRecipe.includes(" --outputStyle=stream"), poolRecipe);
  assert.match(
    read("scripts/nx.sh"),
    /\*" --outputStyle"\*\) exec node_modules\/\.bin\/nx "\$@" ;;/,
  );
  const templateRecipe = recipe("github-live-template");
  assert.ok(
    templateRecipe.includes(
      `${template}={{ if action =~ '^(dry-run|fill)$' { action } else { error(`,
    ),
    templateRecipe,
  );
  for (const [recipe, refused] of [
    ["linear-document-pool", "initialize; echo injected"],
    ["github-live-template", "$(echo injected)"],
  ]) {
    const result = spawnSync("just", ["--dry-run", recipe, refused], {
      cwd: fileURLToPath(new URL("../../", import.meta.url)),
      encoding: "utf8",
    });
    assert.notEqual(result.status, 0, `${recipe} accepted ${refused}`);
    assert.equal(result.stdout, "", `${recipe} ran something for ${refused}`);
    assert.match(result.stderr, /takes dry-run or/, result.stderr);
  }
  assert.ok(templateRecipe.includes("GITHUB_LIVE_SELECTION=run"), templateRecipe);
  assert.ok(
    templateRecipe.includes(
      "--test live -- real_projects_v2_contract_writes_and_leaves_no_residue --exact",
    ),
    templateRecipe,
  );
  for (const [lane, journey] of [
    ["onetaskgraph-linear", "real_linear_applies_every_declared_capability_and_leaves_no_residue"],
    [
      "onetaskgraph-linear-e2e",
      "linear_assets::live_disposable_issue_and_document_assets_copy_and_recopy",
    ],
  ]) {
    const project = JSON.parse(read(`crates/${lane}/project.json`));
    assert.ok(project.targets["live-documents"].options.command.includes(journey), lane);
  }
});

// A cancelled job hands back its Linear leases only if the lanes recorded them where its
// cancellation step looks, so the variable, the directory and the step are held together.
test("each credentialed job records its leases where its cancellation step releases them", () => {
  const variable = read("crates/onetaskgraph-linear/tests/document_lifecycle/lease.rs").match(
    /pub const RECORD_VARIABLE: &str = "([^"]+)";/,
  )[1];
  const directory = "target/linear-leases";
  const release = `cargo run --locked -p onetaskgraph-live-janitor -- --linear --release-leases ${directory}`;
  const jobs = workflows()["ci.yml"].jobs;
  for (const name of ["check", "test-linux"]) {
    const steps = jobs[name].steps;
    const sessions = steps.filter(
      (step) => step.env?.LINEAR_API_KEY && /^just /.test(step.run ?? ""),
    );
    assert.ok(sessions.length > 0, `${name} opens no Linear session`);
    for (const step of sessions) {
      assert.equal(step.env[variable], `\${{ github.workspace }}/${directory}`, name);
    }
    const releases = steps.filter((step) => step.run === release);
    assert.equal(releases.length, 1, `${name} has no step releasing ${directory}`);
    assert.match(releases[0].if, /^cancelled\(\)/, name);
  }
});

// These surfaces select the policy. Whether they also select github-workflows is that
// project's own read set to say: its CD journeys build and package every crate and run the
// justfile's release recipes, so .github/test-inputs.mjs names both, reconciled by ci-contexts.
test("surface edits select document policy", () => {
  const require = createRequire(import.meta.url);
  const manifest = require.resolve("nx/package.json");
  const nx = resolve(dirname(manifest), require("nx/package.json").bin.nx);
  const cwd = fileURLToPath(new URL("../../", import.meta.url));
  for (const file of [
    "justfile",
    "crates/onetaskgraph-linear/project.json",
    "crates/onetaskgraph-linear-e2e/project.json",
  ]) {
    const projects = JSON.parse(
      execFileSync(
        process.execPath,
        [nx, "show", "projects", "--affected", "--files", file, "--json"],
        { cwd, encoding: "utf8" },
      ),
    );
    assert.ok(projects.includes("linear-documents-policy"), `${file} must select its policy`);
  }
  const policyOnly = JSON.parse(
    execFileSync(
      process.execPath,
      [
        nx,
        "show",
        "projects",
        "--affected",
        "--files",
        ".github/linear-documents-policy/policy.test.mjs",
        "--json",
      ],
      { cwd, encoding: "utf8" },
    ),
  );
  assert.ok(policyOnly.includes("linear-documents-policy"));
  assert.ok(
    !policyOnly.includes("github-workflows"),
    "the policy alone must not select publication journeys",
  );
});
