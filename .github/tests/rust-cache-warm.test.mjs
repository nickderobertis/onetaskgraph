import assert from "node:assert/strict";
import { test } from "node:test";
import { existsSync } from "node:fs";
import { parse } from "yaml";
import { HASHED_ENV_PREFIXES } from "../rust-cache-key.mjs";
import { read, root } from "../workflow-graph.mjs";

// Each leg of rust-cache-warm.yml must save exactly the cache its pull-request job restores:
// the same shared key, the same toolchain and rust-cache inputs on the same runner, and the
// same build — or the save is a cache nobody reads and every pull request builds cold.
// A real save and restore needs GitHub's cache service, so the key is proven in two halves
// that need none: this parity of every input rust-cache keys on, beside rust-cache.test.mjs
// running the real action's key computation, and scripts/check-warm-rust-cache-reuse.sh,
// which builds the job's units over what the warmer built and fails on any recompiled.

const FIXTURE = ".github/tests/fixtures/ci-speedup-test-jobs.yml";
const SETUP = "actions-rust-lang/setup-rust-toolchain@";
const UNINSTALL = "rustup toolchain uninstall stable";
const WARM = /^bash scripts\/warm-rust-cache\.sh(?: (--instrumented))?$/;

function sources() {
  const ciJob = new URL("scripts/ci-job.sh", root);
  return {
    warmer: parse(read(".github/workflows/rust-cache-warm.yml")),
    ci: parse(read(".github/workflows/ci.yml")),
    fixture: existsSync(new URL(FIXTURE, root)) ? parse(read(FIXTURE)) : { jobs: {} },
    ciJob: existsSync(ciJob) ? read("scripts/ci-job.sh") : undefined,
  };
}

function resolve(value, matrix) {
  if (typeof value === "string")
    return value.replace(/\$\{\{\s*matrix\.([A-Za-z0-9_-]+)\s*\}\}/g, (_, name) => {
      if (!Object.hasOwn(matrix, name))
        throw Error(`rust-cache-warm.yml names matrix.${name}, which its leg does not define`);
      return String(matrix[name]);
    });
  if (Array.isArray(value)) return value.map((entry) => resolve(entry, matrix));
  if (value && typeof value === "object")
    return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, resolve(v, matrix)]));
  return value;
}

function legs(warmer) {
  const found = [];
  for (const [id, job] of Object.entries(warmer.jobs ?? {})) {
    if (!job.steps?.some((step) => step.uses?.startsWith(SETUP))) continue;
    const combinations = job.strategy?.matrix?.include ?? [{}];
    for (const matrix of combinations) {
      const resolved = resolve(job, matrix);
      const setup = resolved.steps.find((step) => step.uses?.startsWith(SETUP));
      found.push({ id, job: resolved, key: setup.with?.["cache-shared-key"] });
    }
  }
  return found;
}

function hashedEnv(...levels) {
  const merged = Object.assign({}, ...levels.map((env) => env ?? {}));
  return Object.fromEntries(
    Object.entries(merged)
      .filter(([name]) => HASHED_ENV_PREFIXES.some((prefix) => name.startsWith(prefix)))
      .sort(([a], [b]) => a.localeCompare(b)),
  );
}

function toolchain(job) {
  const steps = job.steps ?? [];
  const index = steps.findIndex((step) => step.uses?.startsWith(SETUP));
  if (index < 0) return undefined;
  const { "cache-shared-key": _key, "cache-save-if": _save, ...inputs } = steps[index].with ?? {};
  return {
    uses: steps[index].uses,
    inputs,
    key: steps[index].with?.["cache-shared-key"],
    hasKeyInput: steps[index].with?.["cache-key"] !== undefined,
    uninstalled: steps[index - 1]?.run === UNINSTALL,
  };
}

function installs(job) {
  return (job.steps ?? [])
    .filter((step) => step.uses?.startsWith("taiki-e/install-action@"))
    .flatMap((step) => String(step.with?.tool ?? "").split(","))
    .map((tool) => tool.trim())
    .filter(Boolean);
}

// The phases a pull-request job runs: its arm of scripts/ci-job.sh once that exists, else the
// fixture's copy of it.
function phases(id, fixtureJob, ciJob) {
  if (ciJob !== undefined) {
    const arm = ciJob.match(new RegExp(`\\n  ${id}\\)\\n[\\s\\S]*?phases="([\\s\\S]*?)"\\n`));
    if (arm) return arm[1];
  }
  return fixtureJob?.phases;
}

function targets(text) {
  return text
    .split("\n")
    .map((line) => line.trim().split(/\s+/))
    .filter((fields) => fields.length >= 3)
    .flatMap(([, kind, list]) =>
      kind === "run" ? [list.split(":")[1]] : list.split(",").map((t) => t.trim()),
    );
}

function problems({ warmer, ci, fixture, ciJob }) {
  const found = [];
  const all = legs(warmer);
  if (!all.length) found.push("rust-cache-warm.yml has no leg running setup-rust-toolchain");
  for (const id of Object.keys(fixture.jobs ?? {}))
    if (ci.jobs?.[id])
      found.push(`${id} is in ci.yml and in ${FIXTURE}; delete the fixture now ci.yml has the job`);
  for (const leg of all) {
    const name = `${leg.id} (${leg.job["runs-on"]})`;
    if (!leg.key) {
      found.push(`${name} saves under no cache-shared-key`);
      continue;
    }
    const pr = ci.jobs?.[leg.key] ?? fixture.jobs?.[leg.key];
    if (!pr) {
      found.push(`${name} saves key ${leg.key}, which no job in ci.yml or ${FIXTURE} restores`);
      continue;
    }
    const mine = toolchain(leg.job);
    const theirs = toolchain(pr);
    if (!theirs) {
      found.push(`${leg.key} runs no setup-rust-toolchain step, so restores no Rust cache`);
      continue;
    }
    // rust-cache names a cache by its shared key, else by the job id.
    const restored = theirs.key ?? (theirs.hasKeyInput ? undefined : leg.key);
    if (restored !== leg.key)
      found.push(`${name} saves key ${leg.key}, but ${leg.key} restores ${restored}`);
    if (leg.job["runs-on"] !== pr["runs-on"])
      found.push(`${name} runs on ${leg.job["runs-on"]}, ${leg.key} on ${pr["runs-on"]}`);
    if (mine.uses !== theirs.uses)
      found.push(`${name} sets up the toolchain with ${mine.uses}, ${leg.key} with ${theirs.uses}`);
    try {
      assert.deepEqual(mine.inputs, theirs.inputs);
    } catch {
      found.push(
        `${name} gives setup-rust-toolchain ${JSON.stringify(mine.inputs)}, ${leg.key} ${JSON.stringify(theirs.inputs)}`,
      );
    }
    if (!mine.uninstalled || !theirs.uninstalled)
      found.push(`${name} and ${leg.key} must both remove floating stable before the cache step`);
    try {
      assert.deepEqual(hashedEnv(warmer.env, leg.job.env), hashedEnv(ci.env, pr.env));
    } catch {
      found.push(`${name} and ${leg.key} differ in an env var rust-cache hashes into the key`);
    }
    const builds = (leg.job.steps ?? [])
      .map((step) => step.run?.trim().match(WARM))
      .filter(Boolean);
    if (builds.length !== 1) {
      found.push(`${name} must run scripts/warm-rust-cache.sh exactly once`);
      continue;
    }
    const instrumented = builds[0][1] === "--instrumented";
    const prPhases = phases(leg.key, fixture.jobs?.[leg.key], ciJob);
    if (prPhases === undefined) {
      found.push(`${leg.key}'s phases are in neither scripts/ci-job.sh nor ${FIXTURE}`);
      continue;
    }
    const run = targets(prPhases);
    if (instrumented !== run.includes("coverage"))
      found.push(
        `${name} builds ${instrumented ? "instrumented" : "uninstrumented"}, but ${leg.key} ${run.includes("coverage") ? "runs" : "does not run"} coverage`,
      );
    if (!run.includes("distribution-test"))
      found.push(`${name} builds the distribution journey's release, which ${leg.key} never runs`);
    if (instrumented)
      for (const tool of ["cargo-llvm-cov", "cargo-nextest"])
        if (!installs(leg.job).includes(tool) || !installs(pr).includes(tool))
          found.push(`${name} and ${leg.key} must both install ${tool} to build instrumented`);
  }
  return found;
}

test("every warmer leg saves what its pull-request job restores", () => {
  const all = sources();
  assert.deepEqual(problems(all), []);
  assert.deepEqual(
    legs(all.warmer).map((leg) => `${leg.key}:${leg.job["runs-on"]}`),
    ["test-linux:ubuntu-latest", "test-windows:windows-latest"],
  );
  // A push to main schedules no Mac job: the macOS cache is warmed by its own pull requests.
  for (const [id, job] of Object.entries(all.warmer.jobs))
    for (const matrix of job.strategy?.matrix?.include ?? [{}])
      assert.doesNotMatch(String(resolve(job["runs-on"], matrix)), /mac/i, id);
});

test("a leg differing from its job in key, toolchain or build is refused", () => {
  const original = sources();
  const mutate = (edit) => {
    const changed = structuredClone(original);
    edit(changed);
    return problems(changed);
  };
  const warm = (all) => all.warmer.jobs.warm;
  const leg = (all, id) => warm(all).strategy.matrix.include.find((entry) => entry.job === id);
  const linux = (all) => leg(all, "test-linux");
  const job = (all, id) => all.ci.jobs[id] ?? all.fixture.jobs[id];
  const setup = (steps) => steps.find((step) => step.uses?.startsWith(SETUP));
  const cases = {
    "another key": [
      /saves key test-linux-other, which no job/,
      (all) => {
        linux(all).job = "test-linux-other";
      },
    ],
    "the pull-request job's key": [
      /saves key test-windows, but test-windows restores windows/,
      (all) => {
        setup(job(all, "test-windows").steps).with["cache-shared-key"] = "windows";
      },
    ],
    "another toolchain": [
      /warm \(ubuntu-latest\) gives setup-rust-toolchain/,
      (all) => {
        setup(warm(all).steps).with.toolchain = "nightly";
      },
    ],
    "another cache input": [
      /warm \(windows-latest\) gives setup-rust-toolchain/,
      (all) => {
        setup(job(all, "test-windows").steps).with["cache-workspaces"] = "crates";
      },
    ],
    "another runner": [
      /runs on ubuntu-24\.04-arm, test-linux on ubuntu-latest/,
      (all) => {
        linux(all).os = "ubuntu-24.04-arm";
      },
    ],
    "a hashed env var": [
      /differ in an env var rust-cache hashes/,
      (all) => {
        warm(all).env = { RUSTFLAGS: "-C debug-assertions" };
      },
    ],
    "floating stable kept": [
      /must both remove floating stable/,
      (all) => {
        const steps = warm(all).steps;
        steps.splice(
          steps.findIndex((step) => step.run === UNINSTALL),
          1,
        );
      },
    ],
    "uninstrumented on Linux": [
      /builds uninstrumented, but test-linux runs coverage/,
      (all) => {
        linux(all).build = "";
      },
    ],
    "instrumented on Windows": [
      /builds instrumented, but test-windows does not run coverage/,
      (all) => {
        leg(all, "test-windows").build = "--instrumented";
      },
    ],
    "the job no longer runs coverage": [
      /builds instrumented, but test-linux does not run coverage/,
      (all) => {
        all.ciJob = undefined;
        all.fixture.jobs["test-linux"].phases =
          "1 affected test -\n1 run scripts:distribution-test -";
      },
    ],
    "no build": [
      /must run scripts\/warm-rust-cache\.sh exactly once/,
      (all) => {
        const steps = warm(all).steps;
        steps.splice(
          steps.findIndex((step) => step.run?.startsWith("bash scripts/warm-rust-cache.sh")),
          1,
        );
      },
    ],
    "the job in both files": [
      /test-windows is in ci\.yml and in/,
      (all) => {
        all.ci.jobs["test-windows"] = structuredClone(all.fixture.jobs["test-windows"]);
      },
    ],
    "the job in neither file": [
      /saves key test-windows, which no job/,
      (all) => {
        delete all.fixture.jobs["test-windows"];
        delete all.ci.jobs["test-windows"];
      },
    ],
  };
  // A leg naming a matrix field it does not define is refused, not resolved to nothing.
  assert.throws(
    () => mutate((all) => delete linux(all).build),
    /names matrix\.build, which its leg does not define/,
  );
  for (const [name, [expected, edit]] of Object.entries(cases)) {
    const found = mutate(edit);
    assert.ok(
      found.some((problem) => expected.test(problem)),
      `${name} must be refused with ${expected}: ${JSON.stringify(found)}`,
    );
  }
});
