import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { test } from "node:test";
import { parse } from "yaml";

// The pull request's four jobs, read from the workflow files exactly as GitHub reads them.
// Its inputs are .github alone (the github-workflows project's `ci-jobs` target), so a change
// anywhere in this directory re-runs it and nothing outside does.
const directory = new URL("../workflows/", import.meta.url);
const reusableLock = new URL("../reusable-lock.json", import.meta.url);

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
// Every pull request, and for test-macos every one but release-plz's own release branch: the
// manager's ruling, so the release pull request allocates no Mac runner at all.
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

// Every name a job of these workflows reports as a check context.
function contexts(all) {
  const names = [];
  for (const workflow of Object.values(all))
    for (const [id, job] of Object.entries(workflow?.jobs ?? {})) {
      if (typeof job?.uses === "string" && job.uses.includes("visual-docs-reusable.yml")) {
        const lock = JSON.parse(readFileSync(reusableLock, "utf8"));
        const pinned = Object.keys(lock).some((ref) => ref.endsWith(job.uses));
        if (pinned && !job.name)
          names.push(`${id} / report (x86_64, shots/current, shots/verify, Visual docs)`);
        continue;
      }
      const name = typeof job?.name === "string" ? job.name : id;
      const legs = job?.strategy?.matrix?.os;
      if (Array.isArray(legs) && name.includes(`\${{ matrix.os }}`))
        for (const os of legs) names.push(name.replace(`\${{ matrix.os }}`, os));
      else names.push(name);
    }
  return names;
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
  const reported = contexts(all);
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
