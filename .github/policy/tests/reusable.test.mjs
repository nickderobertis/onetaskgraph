import assert from "node:assert/strict";
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { read, root, workflows } from "../../workflow-graph.mjs";

function fixture() {
  const directory = mkdtempSync(
    join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir(), "reusable-proof-"),
  );
  const put = (path, text) => {
    const dest = join(directory, path);
    mkdirSync(join(dest, ".."), { recursive: true });
    writeFileSync(dest, text);
  };
  for (const path of [
    ".github/workflow-graph.mjs",
    ".github/telemetry.ts",
    ".github/actions-expressions.mjs",
    ".github/reusable-lock.json",
    ".github/reusable/screencomp-v0.4.8.yml",
    "screencomp.toml",
  ])
    put(path, read(path));
  symlinkSync(
    fileURLToPath(new URL(".github/node_modules", root)),
    join(directory, ".github/node_modules"),
    process.platform === "win32" ? "junction" : "dir",
  );
  const call = workflows()["visual-docs.yml"].jobs["visual-docs"].uses;
  return {
    directory,
    put,
    call,
    run(workflow) {
      put("workflow.json", JSON.stringify(workflow));
      return spawnSync(
        process.execPath,
        [
          "--input-type=module",
          "-e",
          `import {jobs} from './.github/workflow-graph.mjs'; import {readFileSync} from 'node:fs'; console.log(JSON.stringify(jobs(JSON.parse(readFileSync('workflow.json','utf8')),{event:'pull_request',action:'opened'},{macosRequired:true,reason:'ordinary'})));`,
        ],
        { cwd: directory, encoding: "utf8" },
      );
    },
    close() {
      rmSync(directory, { recursive: true, force: true });
    },
  };
}
test("local reusable calls resolve actual files, reject unsafe paths and detect recursion", () => {
  const f = fixture();
  try {
    f.put(
      ".github/workflows/child.yml",
      "on: workflow_call\njobs:\n  proof:\n    runs-on: ubuntu-latest\n",
    );
    const parent = { jobs: { call: { uses: "./.github/workflows/child.yml" } } };
    const result = f.run(parent);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(JSON.parse(result.stdout)[0].job, "call / proof");
    const unsafe = f.run({ jobs: { call: { uses: "./.github/workflows/../../outside.yml" } } });
    assert.notEqual(unsafe.status, 0);
    assert.match(unsafe.stderr, /Unresolved local reusable/);
    f.put(
      ".github/workflows/child.yml",
      "on: workflow_call\njobs:\n  loop:\n    uses: ./.github/workflows/child.yml\n",
    );
    const recursion = f.run(parent);
    assert.notEqual(recursion.status, 0);
    assert.match(recursion.stderr, /Recursive reusable/);
  } finally {
    f.close();
  }
});
test("pinned reusable source rejects malformed locks, content mismatch and upstream-byte drift", () => {
  for (const kind of ["path", "digest", "upstream", "missing-upstream", "empty-upstream"]) {
    const f = fixture();
    try {
      const lock = JSON.parse(read(".github/reusable-lock.json"));
      const entry = lock[f.call];
      if (kind === "path") entry.path = "../../outside.yml";
      else if (kind === "missing-upstream") delete entry.upstreamSha256;
      else if (kind === "empty-upstream") entry.upstreamSha256 = "";
      else {
        const source = `${read(entry.path)}\n# changed upstream\n`;
        f.put(entry.path, source);
        if (kind === "upstream") entry.sha256 = createHash("sha256").update(source).digest("hex");
      }
      f.put(".github/reusable-lock.json", JSON.stringify(lock));
      const result = f.run({
        jobs: {
          visual: { uses: f.call, with: workflows()["visual-docs.yml"].jobs["visual-docs"].with },
        },
      });
      assert.notEqual(result.status, 0);
      assert.match(
        result.stderr,
        ["path", "missing-upstream", "empty-upstream"].includes(kind)
          ? /Unresolved reusable lock/
          : kind === "digest"
            ? /digest mismatch/
            : /upstream source drift/,
      );
    } finally {
      f.close();
    }
  }
});
test("architecture inputs and the actual matrix-output command fail explicitly when unresolved", () => {
  for (const arches of ["[]", '["unsupported"]']) {
    const f = fixture();
    try {
      f.put("screencomp.toml", `[capture]\narches = ${arches}\n`);
      const result = f.run({
        jobs: {
          visual: { uses: f.call, with: workflows()["visual-docs.yml"].jobs["visual-docs"].with },
        },
      });
      assert.notEqual(result.status, 0);
      assert.match(result.stderr, /Unresolved capture architecture/);
    } finally {
      f.close();
    }
  }
  for (const mutation of ["missing-step", "missing-output"]) {
    const f = fixture();
    try {
      // The changed external workflow is real declarative input to the unchanged reader;
      // its Bash command still executes at the same subprocess boundary.
      const script = `import {parse} from './.github/node_modules/yaml/dist/index.js'; import {readFileSync,writeFileSync} from 'node:fs'; const w=parse(readFileSync('.github/reusable/screencomp-v0.4.8.yml','utf8')); ${mutation === "missing-step" ? "w.jobs.arches.steps=w.jobs.arches.steps.filter(s=>s.id!=='read');" : "w.jobs.arches.steps.find(s=>s.id==='read').run='echo no-matrix > \"$GITHUB_OUTPUT\"';"} writeFileSync('workflow.json',JSON.stringify(w));`;
      execFileSync(process.execPath, ["--input-type=module", "-e", script], { cwd: f.directory });
      const result = spawnSync(
        process.execPath,
        [
          "--input-type=module",
          "-e",
          `import {jobs} from './.github/workflow-graph.mjs'; import {readFileSync} from 'node:fs'; jobs(JSON.parse(readFileSync('workflow.json','utf8')),{event:'pull_request',action:'opened'},{macosRequired:true});`,
        ],
        { cwd: f.directory, encoding: "utf8" },
      );
      assert.notEqual(result.status, 0);
      assert.match(
        result.stderr,
        mutation === "missing-step"
          ? /Unresolved reusable arches command/
          : /Unresolved reusable arches output/,
      );
    } finally {
      f.close();
    }
  }
});

test("reusable call validates required, unknown and typed inputs before resolving child jobs", () => {
  const f = fixture();
  try {
    f.put(
      ".github/workflows/child.yml",
      "on:\n  workflow_call:\n    inputs:\n      enabled:\n        type: boolean\n        required: true\njobs:\n  proof:\n    runs-on: ubuntu-latest\n",
    );
    const parent = (withInputs) => ({
      jobs: { call: { uses: "./.github/workflows/child.yml", with: withInputs } },
    });
    assert.equal(f.run(parent({ enabled: true })).status, 0);
    for (const [inputs, message] of [
      [{}, /Missing required reusable input/],
      [{ enabled: "true" }, /Invalid reusable input type/],
      [{ enabled: true, extra: true }, /Unknown reusable input/],
    ]) {
      const result = f.run(parent(inputs));
      assert.notEqual(result.status, 0);
      assert.match(result.stderr, message);
    }
  } finally {
    f.close();
  }
});

test("reusable inputs enforce declarations and preserve typed optional defaults", () => {
  const f = fixture();
  try {
    const path = ".github/workflows/defaults.yml";
    const parent = { jobs: { call: { uses: `./${path}` } } };
    for (const declaration of ["false", "''", "{enabled: {type: boolean, required: 'false'}}"]) {
      f.put(
        path,
        `on:\n  workflow_call:\n    inputs: ${declaration}\njobs:\n  proof:\n    runs-on: ubuntu-latest\n`,
      );
      const result = f.run(parent);
      assert.notEqual(result.status, 0);
      assert.match(result.stderr, /Unresolved (?:reusable input|workflow inputs)/);
    }
    f.put(
      path,
      `on:\n  workflow_call:\n    inputs:\n      text: {type: string}\n      enabled: {type: boolean}\n      count: {type: number}\n      given: {type: string, default: provided}\njobs:\n  proof:\n    runs-on: ubuntu-latest\n    name: "\${{ inputs.enabled && 'enabled' || 'disabled' }} (\${{ inputs.count }})\${{ inputs.text }}-\${{ inputs.given }}"\n`,
    );
    const result = f.run(parent);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(JSON.parse(result.stdout)[0].name, "call / disabled (0)-provided");
  } finally {
    f.close();
  }
});

test("reusable paths reject encoded and backslash traversal and require a callable declaration", () => {
  const f = fixture();
  try {
    const source = JSON.parse(read(".github/reusable-lock.json"))[f.call].path;
    f.put(".github/outside.yml", read(source));
    const inputs = workflows()["visual-docs.yml"].jobs["visual-docs"].with;
    for (const call of [
      "./.github/workflows/%2e%2e/outside.yml",
      "./.github/workflows/..\\outside.yml",
    ])
      assert.match(
        f.run({ jobs: { call: { uses: call, with: inputs } } }).stderr,
        /Unresolved local reusable/,
      );
    for (const path of [
      ".github/reusable/%2e%2e/outside.yml",
      ".github/reusable/..\\outside.yml",
    ]) {
      const lock = JSON.parse(read(".github/reusable-lock.json"));
      lock[f.call].path = path;
      f.put(".github/reusable-lock.json", JSON.stringify(lock));
      assert.match(
        f.run({ jobs: { call: { uses: f.call, with: inputs } } }).stderr,
        /Unresolved reusable lock/,
      );
    }
    f.put(
      ".github/workflows/not-callable.yml",
      "on: push\njobs:\n  proof:\n    runs-on: ubuntu-latest\n",
    );
    const result = f.run({ jobs: { call: { uses: "./.github/workflows/not-callable.yml" } } });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /workflow_call declaration/);
  } finally {
    f.close();
  }
});
