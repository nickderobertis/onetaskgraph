import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { test } from "node:test";
import { parse } from "yaml";
import { graph, readSet } from "../workflow-graph.mjs";

// The pull request's four jobs, read from the workflow files exactly as GitHub reads them.
// Its inputs (the github-workflows project's `ci-jobs` target) are this directory and the files
// outside it the run graph reads, so a change to any of them re-runs it and nothing else does.
const directory = new URL("../workflows/", import.meta.url);

function definitions() {
  return Object.fromEntries(
    readdirSync(directory)
      .filter((name) => name.endsWith(".yml"))
      .map((name) => [name, parse(readFileSync(new URL(name, directory), "utf8"))]),
  );
}

const FOUR = {
  lint: "ubuntu-latest",
  "test-linux": "ubuntu-latest",
  "test-windows": "windows-latest",
  "test-macos": "macos-latest",
};
const TESTS = ["test-linux", "test-windows", "test-macos"];
// Every pull request, and for test-macos every one but release-plz's own release branch, so
// the release pull request allocates no Mac runner at all.
const PULL_REQUEST = "github.event_name == 'pull_request'";
const CONDITIONS = {
  lint: PULL_REQUEST,
  "test-linux": PULL_REQUEST,
  "test-windows": PULL_REQUEST,
  "test-macos": `${PULL_REQUEST} && !(github.event.pull_request.head.repo.full_name == github.repository && startsWith(github.head_ref, 'release-plz-'))`,
};
const SCREENCOMP = [
  /^nickderobertis\/screencomp@v\d+\.\d+\.\d+$/,
  /^nickderobertis\/screencomp\/visual-docs@v\d+\.\d+\.\d+$/,
  /^nickderobertis\/screencomp\/visual-docs-pages@v\d+\.\d+\.\d+$/,
  /^nickderobertis\/screencomp\/visual-docs-aggregate@v\d+\.\d+\.\d+$/,
];
// llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] Branch protection's required set is not this repository's to hold: its source is the create-repo skill's setup_github_governance.py, and the orchestration repository reconciles its inventory of this merge path against GitHub on every gate run (AGENTS.md, "Commits, releases, and merging"). A pull request's token cannot read branch protection, so this is the set the workflow must keep producing, held to the workflow below.
const REQUIRED_CONTEXTS = [
  "check (ubuntu-latest)",
  "check (windows-latest)",
  "deny",
  "llmlint",
  "pr-title",
  "install path (ubuntu-latest)",
  "install path (windows-latest)",
  "visual-docs / report (x86_64, shots/current, shots/verify, Visual docs)",
];
// llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate]
const LOCKFILES = [
  "Cargo.lock",
  "bun.lock",
  "uv.lock",
  "sdks/python/uv.lock",
  ".github/bun.lock",
  "rust-toolchain.toml",
];
// What each cache restores, and every input that produced it (a lockfile as hashFiles names
// it). A cache whose path is not
// named here is refused: its inputs have to be stated before it can be restored.
const CACHES = [
  {
    path: /onetaskgraph\/tools\/release-plz/,
    produced: ["runner.os", "runner.arch", "steps.pins.outputs.release-plz"],
  },
  {
    path: /cargo-prune-superseded/,
    produced: [
      "runner.os",
      "runner.arch",
      "steps.pins.outputs.prune-superseded",
      "hashFiles('rust-toolchain.toml')",
    ],
  },
  { path: /^\.nx\/cache$/, produced: ["runner.os", "runner.arch", ...LOCKFILES] },
];

const steps = (job) => (Array.isArray(job?.steps) ? job.steps : []);
const uses = (step) => (typeof step?.uses === "string" ? step.uses : "");

// Every check context a source pull request's runs report, resolved as workflow-graph.mjs
// resolves a run: reusable workflows through their locked definitions, every matrix
// dimension, each job's display name. A tree it cannot resolve reports none.
function reportedOnPullRequest(all) {
  const scenario = {
    event: "pull_request",
    action: "synchronize",
    headRef: "feature",
    fork: false,
  };
  try {
    return graph(scenario, { macosRequired: true, reason: "a source change" }, all).flatMap((run) =>
      run.rows.map((row) => row.name),
    );
  } catch {
    return [];
  }
}

export function problems(all) {
  const found = [];
  const ci = all["ci.yml"];
  const jobs = ci?.jobs ?? {};
  const pullRequest = ci?.on?.pull_request;
  if (pullRequest === undefined || pullRequest?.types)
    found.push("ci.yml must run on every pull_request event, with no `types` filter");
  for (const [id, runner] of Object.entries(FOUR)) {
    const job = jobs[id];
    if (!job) {
      found.push(`ci.yml has no job \`${id}\``);
      continue;
    }
    if (job.name !== undefined) found.push(`${id} overrides its name; its check context is its id`);
    if (job["timeout-minutes"] !== 30) found.push(`${id} has timeout-minutes other than 30`);
    if (job.needs !== undefined) found.push(`${id} has a needs edge`);
    if (job["runs-on"] !== runner) found.push(`${id} runs on ${job["runs-on"]}, not ${runner}`);
    if (job.if !== CONDITIONS[id])
      found.push(`${id} must run on every pull_request event and its condition says otherwise`);
    if (job.strategy !== undefined) found.push(`${id} is a matrix, which splits it into jobs`);
    const recipe = steps(job).filter((step) => step.run === `just ci-${id}`);
    if (recipe.length !== 1) found.push(`${id} does not run its checks through \`just ci-${id}\``);
  }
  for (const [file, workflow] of Object.entries(all))
    for (const [id, job] of Object.entries(workflow?.jobs ?? {}))
      if (file === "ci.yml" && typeof job?.uses === "string" && job.uses.includes("screencomp"))
        found.push(`ci.yml job ${id} runs the visual docs through the reusable workflow`);
  const lint = jobs.lint;
  for (const action of SCREENCOMP)
    if (!steps(lint).some((step) => action.test(uses(step))))
      found.push(`lint does not run ${action.source} as a step`);
  for (const [id, job] of Object.entries(jobs))
    if (
      id !== "lint" &&
      steps(job).some((step) => uses(step).startsWith("nickderobertis/screencomp"))
    )
      found.push(`the visual docs steps left lint for ${id}`);
  if (!steps(lint).some((step) => String(step.run ?? "").includes("scripts/screenshots.sh")))
    found.push("lint does not capture the visual docs");
  const granted = lint?.permissions ?? {};
  for (const [scope, level] of [
    ["contents", "write"],
    ["pull-requests", "write"],
    ["pages", "read"],
  ])
    if (granted[scope] !== level) found.push(`lint does not grant itself ${scope}: ${level}`);
  for (const id of TESTS) {
    const decided = steps(jobs[id]).findIndex(
      (step) =>
        step.id === "release" &&
        String(step.run ?? "").includes(".github/release-shortcut.mjs decide"),
    );
    const tested = steps(jobs[id]).findIndex((step) => step.run === `just ci-${id}`);
    if (decided < 0) found.push(`${id} does not consult the release shortcut`);
    else if (tested >= 0 && decided > tested)
      found.push(`${id} consults the shortcut after its tests`);
    if (tested >= 0 && steps(jobs[id])[tested].if !== "steps.release.outputs.tests == 'run'")
      found.push(`${id}'s tests do not wait on the shortcut's answer`);
  }
  if (JSON.stringify(lint ?? {}).includes("release-shortcut"))
    found.push("lint consults the release shortcut");
  // Today's required contexts keep a job behind each of them until protection moves.
  const reported = reportedOnPullRequest(all);
  for (const context of REQUIRED_CONTEXTS)
    if (!reported.includes(context)) found.push(`no job reports the required context ${context}`);
  for (const [file, workflow] of Object.entries(all))
    for (const [id, job] of Object.entries(workflow?.jobs ?? {}))
      for (const step of steps(job)) {
        if (!uses(step).startsWith("actions/cache")) continue;
        const paths = String(step.with?.path ?? "")
          .split("\n")
          .map((line) => line.trim().replace(/^~\//, ""))
          .filter(Boolean);
        for (const keyName of ["key", "restore-keys"]) {
          const key = String(step.with?.[keyName] ?? "");
          if (keyName === "restore-keys" && !key) continue;
          for (const path of paths) {
            const declared = CACHES.find((cache) => cache.path.test(path));
            if (!declared) {
              found.push(`${file} ${id} caches ${path}, whose inputs no rule here states`);
              continue;
            }
            for (const input of declared.produced)
              if (!key.includes(LOCKFILES.includes(input) ? `'${input}'` : input))
                found.push(`${file} ${id} keys ${path} (${keyName}) without ${input}`);
          }
        }
      }
  found.push(...warmerProblems(all, jobs));
  return found;
}

// The Rust cache a new pull request can restore is the one rust-cache-warm.yml saves on main,
// under the shared key each test job restores: a push to main alone, one leg per test job —
// `warm`'s matrix for Linux and Windows, and `warm-macos`, which a release merge skips
// (policy-routing.test.mjs drives that decision over real revisions).
const WARMED = {
  "test-linux": "ubuntu-latest",
  "test-windows": "windows-latest",
  "test-macos": "macos-latest",
};
const toolchainOf = (job) =>
  steps(job).find((step) => uses(step).startsWith("actions-rust-lang/setup-rust-toolchain"));
function warmerProblems(all, jobs) {
  const found = [];
  const warmer = all["rust-cache-warm.yml"];
  const warm = warmer?.jobs?.warm;
  if (!warm) return ["rust-cache-warm.yml has no job `warm`"];
  if (
    JSON.stringify(Object.keys(warmer.on ?? {})) !== '["push"]' ||
    JSON.stringify(warmer.on.push?.branches) !== '["main"]'
  )
    found.push("the Rust cache warmer runs on something other than a push to main");
  const legs = [...(warm.strategy?.matrix?.include ?? [])];
  if (toolchainOf(warm)?.with?.["cache-shared-key"] !== `\${{ matrix.job }}`)
    found.push("the Rust cache warmer does not save under its leg's job key");
  const mac = warmer.jobs["warm-macos"];
  if (mac) {
    legs.push({ os: mac["runs-on"], job: toolchainOf(mac)?.with?.["cache-shared-key"] });
    if (!steps(mac).some((step) => String(step.run ?? "").includes("scripts/warm-rust-cache.sh")))
      found.push(
        "the Rust cache warmer's macOS leg does not build through scripts/warm-rust-cache.sh",
      );
  }
  for (const [id, os] of Object.entries(WARMED)) {
    if (toolchainOf(jobs[id])?.with?.["cache-shared-key"] !== id)
      found.push(`${id} does not restore the Rust cache under the shared key ${id}`);
    if (!legs.some((leg) => leg.job === id && leg.os === os))
      found.push(`the Rust cache warmer has no ${os} leg saving ${id}'s cache`);
  }
  if (legs.some((leg) => !Object.values(WARMED).includes(leg.os)))
    found.push("the Rust cache warmer runs on a runner no warmed job uses");
  if (!steps(warm).some((step) => String(step.run ?? "").includes("scripts/warm-rust-cache.sh")))
    found.push("the Rust cache warmer does not build through scripts/warm-rust-cache.sh");
  return found;
}

test("the finished workflows hold the four jobs, today's required contexts and keyed caches", () => {
  assert.deepEqual(problems(definitions()), []);
});

function mutated(change) {
  const all = definitions();
  change(all);
  return problems(all);
}
function refuses(change, expected) {
  const found = mutated(change);
  assert.ok(
    found.some((problem) => expected.test(problem)),
    `expected a problem matching ${expected}, got ${JSON.stringify(found)}`,
  );
}

test("a job missing, renamed, given a needs edge or another timeout is refused", () => {
  for (const id of Object.keys(FOUR)) {
    refuses((all) => delete all["ci.yml"].jobs[id], new RegExp(`no job \`${id}\``));
    refuses(
      (all) => {
        all["ci.yml"].jobs[`${id}-renamed`] = all["ci.yml"].jobs[id];
        delete all["ci.yml"].jobs[id];
      },
      new RegExp(`no job \`${id}\``),
    );
    refuses((all) => {
      all["ci.yml"].jobs[id].name = `${id} (renamed)`;
    }, /overrides its name/);
    refuses((all) => {
      all["ci.yml"].jobs[id].needs = ["deny"];
    }, /needs edge/);
    for (const minutes of [29, 31, 45, undefined])
      refuses((all) => {
        all["ci.yml"].jobs[id]["timeout-minutes"] = minutes;
      }, /timeout-minutes other than 30/);
    refuses((all) => {
      all["ci.yml"].jobs[id].if = "github.event_name == 'push'";
    }, /every pull_request event/);
  }
});

test("the visual docs capture leaving lint, or running through the reusable workflow, is refused", () => {
  refuses((all) => {
    const lint = all["ci.yml"].jobs.lint;
    const moved = lint.steps.filter((step) => uses(step).startsWith("nickderobertis/screencomp"));
    lint.steps = lint.steps.filter((step) => !moved.includes(step));
    all["ci.yml"].jobs["test-linux"].steps.push(...moved);
  }, /left lint for test-linux/);
  refuses((all) => {
    const lint = all["ci.yml"].jobs.lint;
    lint.steps = lint.steps.filter((step) => !uses(step).startsWith("nickderobertis/screencomp"));
    all["ci.yml"].jobs["visual-docs"] = {
      uses: "nickderobertis/screencomp/.github/workflows/visual-docs-reusable.yml@v0.4.8",
    };
  }, /reusable workflow/);
  refuses((all) => {
    delete all["ci.yml"].jobs.lint.permissions.pages;
  }, /pages: read/);
});

test("a test job whose tests skip the release shortcut, or lint consulting it, is refused", () => {
  for (const id of TESTS) {
    refuses(
      (all) => {
        const job = all["ci.yml"].jobs[id];
        job.steps = job.steps.filter((step) => step.id !== "release");
      },
      new RegExp(`${id} does not consult`),
    );
    refuses(
      (all) => {
        const step = all["ci.yml"].jobs[id].steps.find((each) => each.run === `just ci-${id}`);
        delete step.if;
      },
      new RegExp(`${id}'s tests do not wait`),
    );
  }
  refuses((all) => {
    all["ci.yml"].jobs.lint.steps.push({ run: "node .github/release-shortcut.mjs decide" });
  }, /lint consults/);
});

test("a job behind one of today's required contexts going missing is refused", () => {
  const removals = {
    "check (ubuntu-latest)": (all) => {
      all["ci.yml"].jobs.check.strategy.matrix.os = ["windows-latest"];
    },
    "check (windows-latest)": (all) => {
      all["ci.yml"].jobs.check.strategy.matrix.os = ["ubuntu-latest"];
    },
    deny: (all) => delete all["ci.yml"].jobs.deny,
    llmlint: (all) => delete all["ci.yml"].jobs.llmlint,
    "pr-title": (all) => delete all["ci.yml"].jobs["pr-title"],
    "install path (ubuntu-latest)": (all) => delete all["ci.yml"].jobs["install-path"],
    "install path (windows-latest)": (all) => {
      all["ci.yml"].jobs["install-path"].strategy.matrix.os = ["ubuntu-latest"];
    },
    "visual-docs / report (x86_64, shots/current, shots/verify, Visual docs)": (all) =>
      delete all["visual-docs.yml"].jobs["visual-docs"],
  };
  assert.deepEqual(Object.keys(removals).sort(), [...REQUIRED_CONTEXTS].sort());
  for (const [context, remove] of Object.entries(removals))
    refuses(remove, new RegExp(`required context ${context.replace(/[()/]/g, "\\$&")}`));
  refuses((all) => {
    all["visual-docs.yml"].jobs["visual-docs"].name = "Visual docs";
  }, /required context visual-docs/);
});

test("a cache key omitting the lockfile or the pinned tool version it depends on is refused", () => {
  const cacheOf = (all, job, pattern) =>
    all["ci.yml"].jobs[job].steps.find(
      (step) => uses(step).startsWith("actions/cache") && pattern.test(String(step.with.path)),
    );
  for (const job of Object.keys(FOUR)) {
    for (const lockfile of LOCKFILES)
      refuses(
        (all) => {
          const step = cacheOf(all, job, /\.nx\/cache/);
          step.with.key = step.with.key
            .replace(`'${lockfile}', `, "")
            .replace(`, '${lockfile}'`, "");
        },
        new RegExp(`${job} keys \\.nx/cache \\(key\\) without ${lockfile.replace(".", "\\.")}`),
      );
    refuses((all) => {
      const step = cacheOf(all, job, /\.nx\/cache/);
      step.with["restore-keys"] = `nx-${job}-`;
    }, /restore-keys\) without/);
    refuses((all) => {
      const step = cacheOf(all, job, /release-plz/);
      step.with.key = `release-plz-\${{ runner.os }}-\${{ runner.arch }}`;
    }, /without steps\.pins\.outputs\.release-plz/);
    refuses((all) => {
      const step = cacheOf(all, job, /prune-superseded/);
      step.with.key = step.with.key.replace(`-\${{ hashFiles('rust-toolchain.toml') }}`, "");
    }, /without hashFiles\('rust-toolchain\.toml'\)/);
    refuses((all) => {
      const step = cacheOf(all, job, /prune-superseded/);
      step.with.key = step.with.key.replace(`\${{ steps.pins.outputs.prune-superseded }}`, "");
    }, /without steps\.pins\.outputs\.prune-superseded/);
  }
  refuses((all) => {
    all["ci.yml"].jobs.lint.steps.push({
      uses: "actions/cache@v6",
      with: { path: "target", key: `target-\${{ runner.os }}` },
    });
  }, /caches target, whose inputs no rule here states/);
});

test("the spec's cached target is keyed on every file outside .github the run graph reads", () => {
  const project = JSON.parse(readFileSync(new URL("../project.json", import.meta.url), "utf8"));
  const inputs = project.targets["ci-jobs"].inputs;
  const outside = readSet().filter((path) => !path.startsWith(".github/"));
  assert.ok(
    outside.length > 0,
    "the run graph reads nothing outside .github, so this proves nothing",
  );
  for (const path of outside)
    assert.ok(inputs.includes(`{workspaceRoot}/${path}`), `ci-jobs is not keyed on ${path}`);
  for (const input of inputs.filter((entry) => entry.startsWith("{workspaceRoot}/")))
    assert.ok(
      outside.includes(input.slice("{workspaceRoot}/".length)),
      `ci-jobs is keyed on ${input}, which the run graph does not read`,
    );
});

test("the Rust cache warmer parting from the key a test job restores is refused", () => {
  refuses((all) => {
    all["ci.yml"].jobs["test-windows"].steps.find((step) =>
      uses(step).startsWith("actions-rust-lang/setup-rust-toolchain"),
    ).with = {};
  }, /test-windows does not restore the Rust cache under the shared key/);
  refuses((all) => {
    all["rust-cache-warm.yml"].jobs.warm.strategy.matrix.include[0].job = "test-linux-renamed";
  }, /no ubuntu-latest leg saving test-linux's cache/);
  refuses((all) => {
    all["rust-cache-warm.yml"].jobs.warm.strategy.matrix.include.push({
      os: "ubuntu-24.04-arm",
      job: "test-linux",
    });
  }, /a runner no warmed job uses/);
  refuses((all) => {
    delete all["rust-cache-warm.yml"].jobs["warm-macos"];
  }, /no macos-latest leg saving test-macos's cache/);
  refuses((all) => {
    all["rust-cache-warm.yml"].jobs["warm-macos"].steps.at(-1).run = "cargo build";
  }, /macOS leg does not build through scripts\/warm-rust-cache.sh/);
  refuses((all) => {
    all["rust-cache-warm.yml"].on = { push: { branches: ["main"] }, pull_request: null };
  }, /other than a push to main/);
  refuses((all) => {
    delete all["rust-cache-warm.yml"];
  }, /has no job `warm`/);
});
