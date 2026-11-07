import "../../test-setup.mjs";
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { parse } from "yaml";
import { HASHED_ENV_PREFIXES } from "../../rust-cache-key.mjs";
import { evaluate, read } from "../../workflow-graph.mjs";

// Each leg of rust-cache-warm.yml must save exactly the cache its pull-request job restores:
// the same shared key, the same toolchain and rust-cache inputs on the same runner, and the
// same build — or the save is a cache nobody reads and every pull request builds cold.
// A real save and restore needs GitHub's cache service, so the key is proven in two halves
// that need none: this parity of every input rust-cache keys on, beside rust-cache.test.mjs
// running the real action's key computation, and scripts/check-warm-rust-cache-reuse.sh,
// which builds the job's units over what the warmer built and fails on any recompiled.
// The pull-request jobs are ci.yml's own; no stand-in of them is read.

const SETUP = "actions-rust-lang/setup-rust-toolchain@";
const UNINSTALL = "rustup toolchain uninstall stable";
const WARM = /^bash scripts\/warm-rust-cache\.sh(?: (--instrumented))?$/;

const isMapping = (value) => value !== null && typeof value === "object" && !Array.isArray(value);

// Holds a parsed workflow to the shape the helpers below read, naming the file and where it
// breaks rather than surfacing later as a TypeError.
function shaped(file, document) {
  const refuse = (where, what) => {
    throw Error(`${file}: ${where} is not ${what}`);
  };
  if (!isMapping(document)) refuse("the document", "a mapping");
  if (document.env !== undefined && !isMapping(document.env)) refuse("env", "a mapping");
  if (!isMapping(document.jobs)) refuse("jobs", "a mapping");
  for (const [id, job] of Object.entries(document.jobs)) {
    if (!isMapping(job)) refuse(`jobs.${id}`, "a mapping");
    if (job.env !== undefined && !isMapping(job.env)) refuse(`jobs.${id}.env`, "a mapping");
    const include = job.strategy?.matrix?.include;
    if (include !== undefined && !(Array.isArray(include) && include.every(isMapping)))
      refuse(`jobs.${id}.strategy.matrix.include`, "a list of mappings");
    if (job.phases !== undefined && typeof job.phases !== "string")
      refuse(`jobs.${id}.phases`, "a string");
    if (job.steps === undefined) continue;
    if (!Array.isArray(job.steps)) refuse(`jobs.${id}.steps`, "a list");
    job.steps.forEach((step, index) => {
      const where = `jobs.${id}.steps[${index}]`;
      if (!isMapping(step)) refuse(where, "a mapping");
      if (step.with !== undefined && !isMapping(step.with)) refuse(`${where}.with`, "a mapping");
      for (const field of ["uses", "run"])
        if (step[field] !== undefined && typeof step[field] !== "string")
          refuse(`${where}.${field}`, "a string");
    });
  }
  return document;
}

function sources() {
  const load = (file) => shaped(file, parse(read(file)));
  return {
    warmer: load(".github/workflows/rust-cache-warm.yml"),
    ci: load(".github/workflows/ci.yml"),
    ciJob: read("scripts/ci-job.sh"),
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
    .map((tool) => tool.trim().split("@")[0])
    .filter(Boolean);
}

// The phases a pull-request job runs: its arm of scripts/ci-job.sh.
function phases(id, ciJob) {
  const arm = ciJob.match(new RegExp(`\\n  ${id}\\)\\n[\\s\\S]*?phases="([\\s\\S]*?)"\\n`));
  return arm?.[1];
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

function problems({ warmer, ci, ciJob }) {
  const found = [];
  const all = legs(warmer);
  if (!all.length) found.push("rust-cache-warm.yml has no leg running setup-rust-toolchain");
  for (const leg of all) {
    const name = `${leg.id} (${leg.job["runs-on"]})`;
    if (!leg.key) {
      found.push(`${name} saves under no cache-shared-key`);
      continue;
    }
    const pr = ci.jobs?.[leg.key];
    if (!pr) {
      found.push(`${name} saves key ${leg.key}, which no job in ci.yml restores`);
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
    const prPhases = phases(leg.key, ciJob);
    if (prPhases === undefined) {
      found.push(`${leg.key}'s phases are not in scripts/ci-job.sh`);
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
    ["test-linux:ubuntu-latest", "test-windows:windows-latest", "test-macos:macos-latest"],
  );
  // The one Mac job a push to main schedules (AGENTS.md), skipped on a release merge.
  const macs = Object.entries(all.warmer.jobs).filter(([, job]) =>
    (job.strategy?.matrix?.include ?? [{}]).some((matrix) =>
      /mac/i.test(String(resolve(job["runs-on"], matrix))),
    ),
  );
  assert.deepEqual(
    macs.map(([id]) => id),
    ["warm-macos"],
  );
  assert.deepEqual(macs[0][1].needs, ["release-merge"]);
  assert.match(macs[0][1].if, /needs\.release-merge\.outputs\.version-only != 'true'/);
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
  const job = (all, id) => all.ci.jobs[id];
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
        all.ciJob = all.ciJob.replace(
          /(\n {2}test-linux\)\n[\s\S]*?phases=")[\s\S]*?"\n/,
          (_, arm) => `${arm}1 affected test -\n1 run scripts:distribution-test -"\n`,
        );
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
    "the job missing from ci.yml": [
      /saves key test-windows, which no job/,
      (all) => {
        delete all.ci.jobs["test-windows"];
      },
    ],
    "the job's phases missing": [
      /test-macos's phases are not in scripts\/ci-job\.sh/,
      (all) => {
        all.ciJob = all.ciJob.replace("\n  test-macos)\n", "\n  test-macos-gone)\n");
      },
    ],
  };
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

test("a workflow not shaped as one is refused naming where", () => {
  assert.throws(() => shaped("w.yml", null), /w\.yml: the document is not a mapping/);
  assert.throws(() => shaped("w.yml", { jobs: [] }), /w\.yml: jobs is not a mapping/);
  assert.throws(
    () => shaped("w.yml", { jobs: { warm: { steps: {} } } }),
    /w\.yml: jobs\.warm\.steps is not a list/,
  );
  assert.throws(
    () => shaped("w.yml", { jobs: { warm: { steps: [{ run: 1 }] } } }),
    /w\.yml: jobs\.warm\.steps\[0\]\.run is not a string/,
  );
  assert.throws(
    () => shaped("w.yml", { jobs: { warm: { strategy: { matrix: { include: "x" } } } } }),
    /w\.yml: jobs\.warm\.strategy\.matrix\.include is not a list of mappings/,
  );
});

const CLEANUP = "Delete this family's superseded copies";
const TOKEN = `\${{ github.token }}`;
const FAMILIES = {
  "test-linux": ["v0-rust-test-linux-Linux-x64-", "v0-rust-release-plz-Linux-x64-"],
  "test-windows": ["v0-rust-test-windows-Windows_NT-x64-"],
  "test-macos": ["v0-rust-test-macos-Darwin-arm64-"],
};
// create-repo's landing commit, 270bea5d287f6fe6a844fd6b882922eacadf6ecf.
const COPIES = {
  "scripts/actions-cache-prune.mjs": {
    source: "assets/ci/actions-cache-prune.mjs",
    version: 1,
    sha256: "112878b64182200739b5e0c7e34decc3778f0316f0cb05f9bd40f66445d33031",
  },
  "scripts/actions-budget-figures.sh": {
    source: "assets/budgets/actions-budget-figures.sh",
    version: 12,
    sha256: "b77b3b6fdccba79eae1db6523122d22eec80ac0738d5b4df03321db600697d3d",
  },
  "scripts/nx-remote-cache.mjs": {
    source: "assets/ci/nx-remote-cache.mjs",
    version: 8,
    sha256: "e35435fb6ffde524af3d4244dcd67c115fc9d3f350ab3d535124dbcc07518bd0",
  },
};
const versionsOf = (file) => file.replace(/\.[^.]+$/, ".versions");

function shapeSources() {
  const load = (file) => shaped(file, parse(read(file)));
  return {
    ...sources(),
    release: load(".github/workflows/release.yml"),
    budgets: parse(read("budgets.yaml")),
    triggered: Object.fromEntries(
      readdirSync(new URL("../../workflows/", import.meta.url))
        .filter((file) => file.endsWith(".yml"))
        .map((file) => [file, load(`.github/workflows/${file}`)]),
    ),
    copies: Object.fromEntries(
      Object.keys(COPIES).flatMap((file) => [
        [file, read(file)],
        [versionsOf(file), read(versionsOf(file))],
      ]),
    ),
  };
}

function copyProblems({ copies }) {
  const found = [];
  for (const [file, want] of Object.entries(COPIES)) {
    const lines = copies[file].split("\n");
    const header = lines.findIndex((line) => /^(#|\/\/) sha256: /.test(line));
    const stated = lines[header]?.split(" ").at(-1);
    const actual = createHash("sha256")
      .update(lines.slice(header + 1).join("\n"))
      .digest("hex");
    if (actual !== stated) found.push(`${file} was edited: its body does not match its header`);
    if (stated !== want.sha256) found.push(`${file} is not create-repo's landing copy`);
    if (!lines[header - 2]?.endsWith(`create-repo/${want.source}`))
      found.push(`${file} names another source`);
    if (lines[header - 1]?.split(" ").at(-1) !== String(want.version))
      found.push(`${file} is not version ${want.version}`);
    if (copies[versionsOf(file)].trim().split("\n").at(-1) !== `${want.version} ${want.sha256}`)
      found.push(`${versionsOf(file)} does not end at this copy`);
  }
  return found;
}

const hasToken = (value) => /\bgithub\.token\b|\bsecrets\./.test(JSON.stringify(value ?? ""));

function warmProblems({ warmer, budgets }) {
  const found = [];
  try {
    assert.deepEqual(warmer.on, { push: { branches: ["main"] } });
  } catch {
    found.push("rust-cache-warm.yml runs on something other than a push to main");
  }
  if (
    !warmer.concurrency?.group?.includes("github.workflow") ||
    warmer.concurrency?.["cancel-in-progress"] !== false
  )
    found.push("rust-cache-warm.yml lost the group holding it to one run at a time");
  const named = new Set();
  for (const leg of legs(warmer)) {
    const name = `${leg.id} (${leg.job["runs-on"]})`;
    const steps = leg.job.steps;
    const cleanup = steps.at(-1);
    if (cleanup?.name !== CLEANUP) {
      found.push(`${name}'s last ordinary step is not the cleanup step`);
      continue;
    }
    const words = cleanup.run?.trim().split(/\s+/) ?? [];
    const families = words.flatMap((word, at) => (words[at - 1] === "--family" ? [word] : []));
    if (words.slice(0, 2).join(" ") !== "node scripts/actions-cache-prune.mjs")
      found.push(`${name}'s cleanup step does not run the copied script`);
    try {
      assert.deepEqual(families, FAMILIES[leg.key]);
    } catch {
      found.push(`${name}'s cleanup names ${families}, not ${FAMILIES[leg.key]}`);
    }
    for (const family of families) named.add(family);
    if (cleanup.env?.GH_TOKEN !== TOKEN) found.push(`${name}'s cleanup is not handed the token`);
    try {
      assert.deepEqual(leg.job.permissions, { contents: "read", actions: "write" });
    } catch {
      found.push(`${name} must hold contents: read and actions: write and nothing else`);
    }
    for (const step of steps) {
      if (
        step.uses?.startsWith("actions/checkout@") &&
        step.with?.["persist-credentials"] !== false
      )
        found.push(`${name}'s checkout persists credentials`);
      if (step !== cleanup && (hasToken(step.env) || hasToken(step.with) || hasToken(step.run)))
        found.push(`${name} hands the token to '${step.name ?? step.uses}', not the cleanup alone`);
    }
  }
  const budget = budgets.budgets.find((entry) => entry.id === "onetaskgraph-cache-headroom");
  const command = budget?.command ?? [];
  const families = command.flatMap((word, at) => (command[at - 1] === "--family" ? [word] : []));
  try {
    assert.deepEqual(new Set(families), named);
  } catch {
    found.push(`the headroom budget names ${families}, the cleanup steps ${[...named]}`);
  }
  return found;
}

function savingProblems({ ci, release, warmer, triggered }) {
  const found = [];
  for (const [file, document] of [
    ["ci.yml", ci],
    ["release.yml", release],
  ])
    for (const [id, job] of Object.entries(document.jobs))
      for (const step of job.steps ?? []) {
        if (step.uses?.startsWith("Swatinem/rust-cache@"))
          found.push(`${file} ${id} runs rust-cache directly, which saves`);
        if (step.uses?.startsWith(SETUP) && step.with?.["cache-save-if"] !== false)
          found.push(`${file} ${id}'s Rust step could save`);
      }
  // Pull-request jobs keep restoring, lint from test-linux's family.
  for (const id of ["lint", "test-linux", "test-windows", "test-macos"]) {
    const setup = ci.jobs[id]?.steps?.find((step) => step.uses?.startsWith(SETUP));
    const family = id === "lint" ? "test-linux" : id;
    if (setup?.with?.cache === false || setup?.with?.["cache-shared-key"] !== family)
      found.push(`${id}'s Rust step stopped restoring ${family}'s family`);
  }
  const lint = ci.jobs.lint;
  const leg = legs(warmer).find((candidate) => candidate.key === "test-linux");
  if (lint && leg) {
    const [mine, theirs] = [toolchain(leg.job), toolchain(lint)];
    if (lint["runs-on"] !== leg.job["runs-on"]) found.push("lint runs elsewhere than its warm leg");
    if (mine.uses !== theirs?.uses) found.push("lint and its warm leg set up different toolchains");
    if (!theirs?.uninstalled) found.push("lint must remove floating stable before the cache step");
    try {
      assert.deepEqual(theirs?.inputs, mine.inputs);
      assert.deepEqual(hashedEnv(ci.env, lint.env), hashedEnv(warmer.env, leg.job.env));
    } catch {
      found.push("lint's compiler inputs differ from the Linux warm leg's");
    }
  }
  for (const [file, document] of Object.entries(triggered)) {
    const on = Object.keys(document.on ?? {});
    if (!on.some((event) => event.startsWith("pull_request"))) continue;
    if (document.permissions?.actions === "write" || document.permissions === "write-all")
      found.push(`${file} grants actions: write to a workflow a pull request triggers`);
    for (const [id, job] of Object.entries(document.jobs))
      if (job.permissions?.actions === "write" || job.permissions === "write-all")
        found.push(`${file} ${id} holds actions: write, and a pull request triggers it`);
  }
  return found;
}

// Every cache-writing step of a pull-request job, each with the expression deciding whether it
// writes: true when that expression's value means a write.
function cacheWriters(triggered) {
  const writers = [];
  for (const [file, document] of Object.entries(triggered)) {
    if (!Object.keys(document.on ?? {}).some((event) => event.startsWith("pull_request"))) continue;
    for (const [id, job] of Object.entries(document.jobs))
      for (const step of job.steps ?? []) {
        const where = `${file} ${id} ${step.uses ?? step.name}`;
        const add = (expression, writes) => writers.push({ where, expression, writes });
        if (/^actions\/cache(\/save)?@/.test(step.uses ?? ""))
          add(step.if ?? "true", (v) => v === true);
        else if (step.uses?.startsWith("astral-sh/setup-uv@"))
          add(step.with?.["save-cache"] ?? "true", (v) => v === true || v === "true");
        else if (step.uses?.startsWith("oven-sh/setup-bun@"))
          add(step.with?.["no-cache"] ?? "false", (v) => v !== true && v !== "true");
        else if (step.uses?.startsWith("op-nx/github-cache/start-cache-server@"))
          add(step.env?.CACHE_READ_ONLY ?? "''", (v) => !v);
      }
  }
  return writers;
}

function eventContext(name) {
  const recorded = JSON.parse(
    read(`.github/policy/tests/fixtures/actions-cache/events/${name}.json`),
  );
  const dir = mkdtempSync(join(tmpdir(), "cache-trust-"));
  try {
    const event = join(dir, "event.json");
    const output = join(dir, "output");
    writeFileSync(event, JSON.stringify(recorded.event));
    const trust = spawnSync(
      process.execPath,
      [new URL("../../../scripts/nx-remote-cache.mjs", import.meta.url).pathname, "trust"],
      {
        cwd: dir,
        encoding: "utf8",
        env: {
          PATH: process.env.PATH,
          GITHUB_EVENT_NAME: "pull_request",
          GITHUB_EVENT_PATH: event,
          GITHUB_OUTPUT: output,
          GITHUB_REPOSITORY: recorded.repository,
          RUNNER_TEMP: dir,
        },
      },
    );
    assert.equal(trust.status, 0, trust.stderr);
    const outputs = Object.fromEntries(
      readFileSync(output, "utf8")
        .trim()
        .split("\n")
        .map((line) => line.split("=")),
    );
    const step = (id, values) => [id, { outputs: values }];
    return {
      github: { event: recorded.event, repository: recorded.repository },
      steps: Object.fromEntries([
        step("nx-cache-trust", outputs),
        step("release-plz-cache", { "cache-hit": "false" }),
        step("prune-superseded-cache", { "cache-hit": "false" }),
      ]),
    };
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

function guardProblems(triggered, fork, same) {
  const found = [];
  const writers = cacheWriters(triggered);
  if (!writers.length) found.push("no cache-writing step of a pull-request job was read");
  for (const { where, expression, writes } of writers) {
    if (writes(evaluate(expression, fork))) found.push(`${where}: a fork's run writes the cache`);
    if (!writes(evaluate(expression, same))) found.push(`${where}: a same-repository run does not`);
  }
  return found;
}

test("the finished tree holds the copies, the cleanup steps, the saves and the guards", () => {
  const all = shapeSources();
  assert.deepEqual(copyProblems(all), []);
  assert.deepEqual(warmProblems(all), []);
  assert.deepEqual(savingProblems(all), []);
  assert.deepEqual(
    guardProblems(all.triggered, eventContext("fork"), eventContext("same-repository")),
    [],
  );
});

test("each way of losing the copies, the cleanup, the saves or the guards is refused", () => {
  const original = shapeSources();
  const fork = eventContext("fork");
  const same = eventContext("same-repository");
  const check = (all) => [
    ...copyProblems(all),
    ...warmProblems(all),
    ...savingProblems(all),
    ...guardProblems(all.triggered, fork, same),
  ];
  const step = (steps, name) => steps.find((candidate) => candidate.name === name);
  const uses = (steps, prefix) => steps.find((candidate) => candidate.uses?.startsWith(prefix));
  const warm = (all) => all.warmer.jobs.warm;
  const linux = (all) => warm(all).strategy.matrix.include.find((leg) => leg.job === "test-linux");
  const ciWorkflow = (all) => all.triggered["ci.yml"];
  const cases = {
    "a copy edited": [
      /actions-cache-prune\.mjs was edited/,
      (all) => {
        all.copies["scripts/actions-cache-prune.mjs"] += "// edited\n";
      },
    ],
    "another version of a copy": [
      /nx-remote-cache\.mjs is not version 8/,
      (all) => {
        all.copies["scripts/nx-remote-cache.mjs"] = all.copies[
          "scripts/nx-remote-cache.mjs"
        ].replace("// version: 8", "// version: 7");
      },
    ],
    "a versions file behind": [
      /actions-budget-figures\.versions does not end at this copy/,
      (all) => {
        all.copies["scripts/actions-budget-figures.versions"] += "13 feed\n";
      },
    ],
    "a leg without its cleanup": [
      /warm-macos \(macos-latest\)'s last ordinary step is not the cleanup step/,
      (all) => {
        all.warmer.jobs["warm-macos"].steps.pop();
      },
    ],
    "a step after the cleanup": [
      /warm \(ubuntu-latest\)'s last ordinary step is not the cleanup step/,
      (all) => {
        warm(all).steps.push({ run: "true" });
      },
    ],
    "a family gone from a leg": [
      /warm \(ubuntu-latest\)'s cleanup names .*, not /,
      (all) => {
        linux(all).families = "--family v0-rust-test-linux-Linux-x64-";
      },
    ],
    "the budget's families parted from the cleanup": [
      /the headroom budget names/,
      (all) => {
        const budget = all.budgets.budgets.find((b) => b.id === "onetaskgraph-cache-headroom");
        budget.command = budget.command.slice(0, -2);
      },
    ],
    "a warm run on a pull request": [
      /runs on something other than a push to main/,
      (all) => {
        all.warmer.on.pull_request = null;
      },
    ],
    "the concurrency group gone": [
      /lost the group/,
      (all) => {
        delete all.warmer.concurrency;
      },
    ],
    "a leg without actions: write": [
      /must hold contents: read and actions: write/,
      (all) => {
        warm(all).permissions = { contents: "read" };
      },
    ],
    "a checkout persisting credentials": [
      /warm \(windows-latest\)'s checkout persists credentials/,
      (all) => {
        delete uses(warm(all).steps, "actions/checkout@").with["persist-credentials"];
      },
    ],
    "the token handed to another step": [
      /hands the token to 'Build what test-linux builds'/,
      (all) => {
        step(warm(all).steps, `Build what \${{ matrix.job }} builds`).env = { GH_TOKEN: TOKEN };
      },
    ],
    "a pull-request Rust step that can save": [
      /ci\.yml test-macos's Rust step could save/,
      (all) => {
        delete uses(all.ci.jobs["test-macos"].steps, SETUP).with["cache-save-if"];
      },
    ],
    "a tag save": [
      /release\.yml publish's Rust step could save/,
      (all) => {
        delete uses(all.release.jobs.publish.steps, SETUP).with["cache-save-if"];
      },
    ],
    "a pull-request Rust step that stops restoring": [
      /test-windows's Rust step stopped restoring/,
      (all) => {
        uses(all.ci.jobs["test-windows"].steps, SETUP).with.cache = false;
      },
    ],
    "lint on a family of its own": [
      /lint's Rust step stopped restoring test-linux's family/,
      (all) => {
        uses(all.ci.jobs.lint.steps, SETUP).with["cache-shared-key"] = "lint";
      },
    ],
    "lint's compiler inputs parted": [
      /lint's compiler inputs differ/,
      (all) => {
        all.ci.jobs.lint.env.RUSTFLAGS = "-D warnings";
      },
    ],
    "lint keeping floating stable": [
      /lint must remove floating stable/,
      (all) => {
        const steps = all.ci.jobs.lint.steps;
        steps.splice(
          steps.findIndex((candidate) => candidate.run === UNINSTALL),
          1,
        );
      },
    ],
    "actions: write on a pull-request job": [
      /ci\.yml budgets holds actions: write/,
      (all) => {
        ciWorkflow(all).jobs.budgets.permissions.actions = "write";
      },
    ],
    "a save with no guard": [
      /ci\.yml test-linux actions\/cache\/save@v6: a fork's run writes the cache/,
      (all) => {
        uses(ciWorkflow(all).jobs["test-linux"].steps, "actions/cache/save@").if =
          "steps.release-plz-cache.outputs.cache-hit != 'true'";
      },
    ],
    "setup-uv's own cache unguarded": [
      /ci\.yml lint astral-sh\/setup-uv@v7: a fork's run writes the cache/,
      (all) => {
        delete uses(ciWorkflow(all).jobs.lint.steps, "astral-sh/setup-uv@").with["save-cache"];
      },
    ],
    "setup-bun's own cache unguarded": [
      /ci\.yml test-windows oven-sh\/setup-bun@v2: a fork's run writes the cache/,
      (all) => {
        delete uses(ciWorkflow(all).jobs["test-windows"].steps, "oven-sh/setup-bun@").with[
          "no-cache"
        ];
      },
    ],
    "the sidecar writing on a fork": [
      /ci\.yml lint op-nx\/github-cache\/start-cache-server@[0-9a-f]+: a fork's run writes the cache/,
      (all) => {
        uses(
          ciWorkflow(all).jobs.lint.steps,
          "op-nx/github-cache/start-cache-server@",
        ).env.CACHE_READ_ONLY = "''";
      },
    ],
    "a guard refusing the same repository too": [
      /ci\.yml lint actions\/cache\/save@v6: a same-repository run does not/,
      (all) => {
        uses(ciWorkflow(all).jobs.lint.steps, "actions/cache/save@").if = "false";
      },
    ],
  };
  for (const [name, [expected, edit]] of Object.entries(cases)) {
    const changed = structuredClone(original);
    edit(changed);
    const found = check(changed);
    assert.ok(
      found.some((problem) => expected.test(problem)),
      `${name} must be refused with ${expected}: ${JSON.stringify(found)}`,
    );
  }
});

// GitHub's cache REST boundary over one mutable listing: prefix match on `key`, sorted and paged
// as the real listing is, and DELETE by id. Everything else is refused, so a request this repository
// does not expect fails the journey.
async function cacheServer(entries) {
  const seen = { deletes: [], tokens: new Set(), requests: 0 };
  const server = createServer((request, response) => {
    seen.requests++;
    seen.tokens.add(request.headers.authorization);
    const url = new URL(request.url, "http://127.0.0.1");
    const send = (status, body) => {
      response.writeHead(status, {
        "content-type": "application/json",
        "x-ratelimit-remaining": "4000",
      });
      response.end(JSON.stringify(body));
    };
    const deleted = url.pathname.match(/^\/repos\/[^/]+\/[^/]+\/actions\/caches\/(\d+)$/);
    if (request.method === "DELETE" && deleted) {
      const at = entries.findIndex((entry) => String(entry.id) === deleted[1]);
      if (at < 0) return send(404, { message: "Not Found" });
      seen.deletes.push(entries[at].key);
      entries.splice(at, 1);
      return send(200, {});
    }
    if (request.method !== "GET") return send(405, {});
    if (url.pathname.endsWith("/actions/cache/storage-limit"))
      return send(200, { max_cache_size_gb: 10 });
    if (!url.pathname.endsWith("/actions/caches")) return send(404, { message: "Not Found" });
    const prefix = url.searchParams.get("key") ?? "";
    const field = url.searchParams.get("sort") === "size_in_bytes" ? "size_in_bytes" : "created_at";
    const sign = url.searchParams.get("direction") === "asc" ? 1 : -1;
    const sorted = entries
      .filter((entry) => entry.key.startsWith(prefix))
      .sort((a, b) => sign * (a[field] < b[field] ? -1 : a[field] > b[field] ? 1 : 0));
    const size = Number(url.searchParams.get("per_page"));
    const page = Number(url.searchParams.get("page") ?? 1);
    return send(200, {
      total_count: sorted.length,
      actions_caches: sorted.slice((page - 1) * size, page * size),
    });
  });
  await new Promise((done) => server.listen(0, "127.0.0.1", done));
  return { url: `http://127.0.0.1:${server.address().port}`, seen, close: () => server.close() };
}

function runCommand(words, url, additions = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(words[0], words.slice(1), {
      cwd: new URL("../../../", import.meta.url).pathname,
      env: {
        PATH: process.env.PATH,
        HOME: process.env.HOME,
        DBUS_SESSION_BUS_ADDRESS: "disabled:",
        GITHUB_API_URL: url,
        GITHUB_REPOSITORY: "nickderobertis/onetaskgraph",
        GH_TOKEN: "recorded-token",
        ...additions,
      },
    });
    let output = "";
    child.stdout.on("data", (chunk) => {
      output += chunk;
    });
    child.stderr.on("data", (chunk) => {
      output += chunk;
    });
    child.on("error", reject);
    child.on("close", (status) => resolve({ status, output }));
  });
}

const recordedListing = () =>
  JSON.parse(read(".github/policy/tests/fixtures/actions-cache/listing.json")).actions_caches;
const cleanupOf = (warmer, key) =>
  legs(warmer)
    .find((leg) => leg.key === key)
    .job.steps.at(-1)
    .run.trim()
    .split(/\s+/);
const inFamily = (entries, family) => entries.filter((entry) => entry.key.startsWith(family));

test("a warm leg's own cleanup command takes each family's superseded copies and keeps main's newest", async () => {
  const entries = recordedListing();
  const { warmer } = sources();
  const results = entries.filter((entry) => entry.key.startsWith("nx-cache-")).length;
  const foreign = inFamily(entries, FAMILIES["test-windows"][0]).length;
  const [linux] = FAMILIES["test-linux"];
  assert.ok(inFamily(entries, linux).length > 2, "the recording holds superseded copies to take");
  const newest = inFamily(entries, linux)
    .filter((entry) => entry.ref === "refs/heads/main")
    .sort((a, b) => (a.created_at < b.created_at ? 1 : -1))[0];
  const server = await cacheServer(entries);
  try {
    const run = await runCommand(cleanupOf(warmer, "test-linux"), server.url);
    assert.equal(run.status, 0, run.output);
    assert.deepEqual(
      inFamily(entries, linux).map((entry) => entry.id),
      [newest.id],
    );
    assert.ok(inFamily(entries, FAMILIES["test-linux"][1]).length <= 1, run.output);
    assert.equal(entries.filter((entry) => entry.key.startsWith("nx-cache-")).length, results);
    assert.equal(inFamily(entries, FAMILIES["test-windows"][0]).length, foreign);
    assert.match(
      run.output,
      new RegExp(`${linux}: kept ${newest.key}, deleted \\d+, freed \\d+ bytes`),
    );
    assert.deepEqual([...server.seen.tokens], ["Bearer recorded-token"]);
  } finally {
    server.close();
  }
});

test("cycles of cleanup then save, a cancelled run among them, never leave a family over two copies", async () => {
  const { warmer } = sources();
  const entries = recordedListing();
  const server = await cacheServer(entries);
  let id = 90_000_000_000;
  const clock = Date.parse("2026-11-08T00:00:00Z") - id;
  const save = (family) =>
    entries.push({
      id: ++id,
      ref: "refs/heads/main",
      key: `${family}cycle-${id}`,
      created_at: new Date(clock + id).toISOString(),
      last_accessed_at: new Date(clock + id).toISOString(),
      size_in_bytes: 1_000_000,
      version: "cycle",
    });
  const assertBound = (when) => {
    for (const family of Object.values(FAMILIES).flat())
      assert.ok(
        inFamily(entries, family).length <= 2,
        `${when}: ${family} holds ${inFamily(entries, family).length}`,
      );
  };
  try {
    // The first cleanup takes the recorded listing down to each family's newest main copy.
    for (const key of Object.keys(FAMILIES)) await runCommand(cleanupOf(warmer, key), server.url);
    assertBound("after the first cleanup");
    const cycles = [
      ["test-linux", "test-windows", "test-macos"],
      // Cancelled after saving only the Linux leg: the others cleaned up and saved nothing.
      ["test-linux"],
      ["test-linux", "test-windows", "test-macos"],
      // Cancelled again, with the Windows leg alone landing, and a release merge's skip of macOS.
      ["test-windows"],
      ["test-linux", "test-windows"],
      ["test-linux", "test-windows", "test-macos"],
    ];
    for (const [number, saved] of cycles.entries()) {
      for (const key of Object.keys(FAMILIES)) {
        const run = await runCommand(cleanupOf(warmer, key), server.url);
        assert.equal(run.status, 0, run.output);
        assert.ok(
          inFamily(entries, FAMILIES[key][0]).length <= 1,
          `cycle ${number}: ${run.output}`,
        );
        if (saved.includes(key)) save(FAMILIES[key][0]);
        assertBound(`cycle ${number}, ${key}`);
      }
    }
  } finally {
    server.close();
  }
});

test("the registered headroom command reads the recorded listing as budgets.yaml states it", async () => {
  const { warmer } = sources();
  const budget = parse(read("budgets.yaml")).budgets.find(
    (entry) => entry.id === "onetaskgraph-cache-headroom",
  );
  assert.deepEqual(budget.labels, ["release-tail"]);
  assert.equal(budget.threshold, 3.37);
  assert.equal(budget.direction, "min");
  const entries = recordedListing();
  const server = await cacheServer(entries);
  const dir = mkdtempSync(join(tmpdir(), "cache-headroom-"));
  const result = join(dir, "result.json");
  try {
    // Over the recording, as it stood, more than two copies of a named family reads 0.
    const over = await runCommand(budget.command, server.url, { ONEBUDGETSPEC_RESULT: result });
    assert.equal(over.status, 0, over.output);
    assert.equal(JSON.parse(readFileSync(result, "utf8")).value, 0, over.output);
    // After each leg's cleanup every named family holds one copy, so the figure is the room left.
    for (const key of Object.keys(FAMILIES)) await runCommand(cleanupOf(warmer, key), server.url);
    const allowance = 10 * 1024 ** 3;
    const expected =
      (allowance - entries.reduce((total, entry) => total + entry.size_in_bytes, 0)) / 1e9;
    const room = await runCommand(budget.command, server.url, { ONEBUDGETSPEC_RESULT: result });
    assert.equal(room.status, 0, room.output);
    assert.ok(
      Math.abs(JSON.parse(readFileSync(result, "utf8")).value - expected) < 0.001,
      `${readFileSync(result, "utf8")} against ${expected}`,
    );
  } finally {
    server.close();
    rmSync(dir, { recursive: true, force: true });
  }
});
