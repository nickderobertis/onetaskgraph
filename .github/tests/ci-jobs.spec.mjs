import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { definitions, refusals } from "./workflow-definitions.mjs";

// The pull request's four jobs, read from the workflow files exactly as GitHub reads them.
// Its inputs (the github-workflows project's `ci-jobs` target) are this directory alone, so it
// reads nothing outside it: whether the required checks are still reported needs the run
// graph, which reads files outside it, and is ci-contexts.spec.mjs's.

const FOUR = {
  lint: "ubuntu-latest",
  "test-linux": "ubuntu-latest",
  "test-windows": "windows-latest",
  "test-macos": "macos-latest",
};
const TESTS = ["test-linux", "test-windows", "test-macos"];
// Each job's execution limit. The three test jobs hold 40 minutes, by the user's decision of
// 2026-10-31: passing runs reached 29.8 minutes against 30, so ordinary variance cancelled them.
const TIMEOUT_MINUTES = { lint: 30, "test-linux": 40, "test-windows": 40, "test-macos": 40 };
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
    if (job["timeout-minutes"] !== TIMEOUT_MINUTES[id])
      found.push(`${id} has timeout-minutes other than ${TIMEOUT_MINUTES[id]}`);
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
  // test-windows' loopback suites exhaust Windows' default dynamic port range (ci.yml says how).
  const widened = steps(jobs["test-windows"]).findIndex((step) =>
    /netsh int ipv4 set dynamicport tcp start=\d+ num=\d+/.test(String(step.run ?? "")),
  );
  const windowsTests = steps(jobs["test-windows"]).findIndex(
    (step) => step.run === "just ci-test-windows",
  );
  if (widened < 0 || (windowsTests >= 0 && widened > windowsTests))
    found.push("test-windows does not widen the dynamic port range before its tests");
  if (JSON.stringify(lint ?? {}).includes("release-shortcut"))
    found.push("lint consults the release shortcut");
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
  // The cache action hashes every installed Rust version, even the runner's unused
  // floating compiler. Its removal must happen before the restore, in every producer
  // and consumer, and under the same condition as the pinned toolchain step.
  for (const [id, job] of [
    ...Object.keys(FOUR).map((id) => [id, jobs[id]]),
    ["warm", warm],
    ["warm-macos", warmer.jobs["warm-macos"]],
  ]) {
    const toolchain = toolchainOf(job);
    const before = steps(job).slice(0, steps(job).indexOf(toolchain));
    if (
      !before.some(
        (step) => step.run === "rustup toolchain uninstall stable" && step.if === toolchain?.if,
      )
    )
      found.push(`${id} must exclude the unused stable toolchain before restoring the build cache`);
  }
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

test("the finished workflows hold the four jobs and keyed caches", () => {
  assert.deepEqual(problems(definitions()), []);
});

const refuses = refusals(problems);

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
    const limit = TIMEOUT_MINUTES[id];
    for (const minutes of [limit - 1, limit + 1, 45, undefined])
      refuses(
        (all) => {
          all["ci.yml"].jobs[id]["timeout-minutes"] = minutes;
        },
        new RegExp(`${id} has timeout-minutes other than ${limit}`),
      );
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

test("test-windows' tests running without the widened dynamic port range is refused", () => {
  refuses((all) => {
    const job = all["ci.yml"].jobs["test-windows"];
    job.steps = job.steps.filter((step) => !String(step.run ?? "").includes("dynamicport"));
  }, /does not widen the dynamic port range/);
  refuses((all) => {
    const job = all["ci.yml"].jobs["test-windows"];
    const widen = job.steps.findIndex((step) => String(step.run ?? "").includes("dynamicport"));
    job.steps.push(job.steps.splice(widen, 1)[0]);
  }, /does not widen the dynamic port range before its tests/);
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

test("the unused floating compiler cannot invalidate a pinned build cache", () => {
  for (const [file, id] of [
    ...Object.keys(FOUR).map((id) => ["ci.yml", id]),
    ["rust-cache-warm.yml", "warm"],
    ["rust-cache-warm.yml", "warm-macos"],
  ]) {
    refuses((all) => {
      const job = all[file].jobs[id];
      job.steps = job.steps.filter((step) => step.run !== "rustup toolchain uninstall stable");
    }, /must exclude the unused stable toolchain before restoring the build cache/);
  }
  refuses((all) => {
    const job = all["ci.yml"].jobs["test-linux"];
    const at = job.steps.findIndex((step) => step.run === "rustup toolchain uninstall stable");
    job.steps.push(...job.steps.splice(at, 1));
  }, /must exclude the unused stable toolchain before restoring the build cache/);
  refuses((all) => {
    all["ci.yml"].jobs["test-linux"].steps.find(
      (step) => step.run === "rustup toolchain uninstall stable",
    ).if = "false";
  }, /must exclude the unused stable toolchain before restoring the build cache/);
});

// The contract's own inputs are this directory alone: nothing outside it re-runs it, so it may
// read nothing outside it — not by an input, and not through a module that reads the tree.
test("the contract's target is keyed on .github alone and reads nothing outside it", () => {
  const project = JSON.parse(readFileSync(new URL("../project.json", import.meta.url), "utf8"));
  assert.deepEqual(contractInputProblems(project), []);
  const broadened = structuredClone(project);
  broadened.namedInputs.default.push("{workspaceRoot}/screencomp.toml");
  assert.match(contractInputProblems(broadened).join("\n"), /screencomp\.toml/);
  const named = structuredClone(project);
  named.targets["ci-jobs"].inputs = ["workflowGraph"];
  assert.match(contractInputProblems(named).join("\n"), /workspaceRoot/);
  assert.deepEqual(importProblems(import.meta.url), []);
  assert.match(
    importProblems(import.meta.url, 'import { graph } from "../workflow-graph.mjs";').join("\n"),
    /workflow-graph\.mjs/,
  );
});

// Every input `ci-jobs` resolves to, named inputs expanded, that is not under its own root.
function contractInputProblems(project) {
  const expand = (inputs) =>
    inputs.flatMap((input) =>
      typeof input === "string" && project.namedInputs?.[input]
        ? expand(project.namedInputs[input])
        : [input],
    );
  return expand(project.targets["ci-jobs"].inputs ?? ["default"])
    .filter((input) => typeof input !== "string" || !/^!?\{projectRoot\}\//.test(input))
    .map((input) => `ci-jobs is keyed on ${JSON.stringify(input)}, outside .github`);
}

// The spec's relative imports, followed through every local module: each stays in .github and
// none is workflow-graph.mjs, which reads files from the repository root.
function importProblems(url, prepended = "") {
  const found = [];
  const seen = new Set();
  const githubRoot = new URL("../", import.meta.url).href;
  const visit = (module, text) => {
    if (seen.has(module)) return;
    seen.add(module);
    for (const [, specifier] of text.matchAll(
      /^\s*(?:import|export)\b[^;]*?\bfrom\s+"([^"]+)"/gm,
    )) {
      if (!specifier.startsWith(".")) continue;
      const target = new URL(specifier, module).href;
      if (!target.startsWith(githubRoot) || target.endsWith("/workflow-graph.mjs"))
        found.push(`${module.slice(githubRoot.length)} imports ${specifier}`);
      else visit(target, readFileSync(new URL(target), "utf8"));
    }
  };
  visit(url, prepended + readFileSync(new URL(url), "utf8"));
  return found;
}
