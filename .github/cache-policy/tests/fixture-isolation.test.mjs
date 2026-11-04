import "../../test-setup.mjs";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { after, test } from "node:test";
import { fileURLToPath } from "node:url";
import { fixtureEnvironment } from "../../fixture-environment.mjs";

const root = fileURLToPath(new URL("../../../", import.meta.url));
const scratch = mkdtempSync(
  join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir(), "fixture-audit-"),
);
after(() => rmSync(scratch, { recursive: true, force: true }));
function command(program, args, cwd, env = {}) {
  const result = spawnSync(program, args, {
    cwd,
    env: fixtureEnvironment({
      NODE_TEST_CONTEXT: undefined,
      GIT_DIR: undefined,
      GIT_WORK_TREE: undefined,
      GIT_INDEX_FILE: undefined,
      GIT_COMMON_DIR: undefined,
      ...env,
    }),
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
    timeout: 600_000,
  });
  assert.equal(result.status, 0, result.stdout + result.stderr);
  return result.stdout.trim();
}

const tiers = [
  ["shell-clone", "bash fixture-shell-clone.sh"],
  ["shell-entry", "bash fixture-shell-entry.sh"],
  ["python-entry", "python3 fixture-python-entry.py"],
  ["node-entry", "node --test fixture-entry-probe.test.mjs"],
  ["node-environment", "node fixture-environment-probe.mjs"],
  ["node-owned-recording", "node fixture-environment-probe.mjs"],
];
for (const [name, tierCommand] of tiers) {
  test(`nested Nx fixture isolation: ${name}`, { timeout: 660_000 }, (t) => {
    if (process.platform === "win32") {
      t.skip("shell clone fixtures require Unix symlinks");
      return;
    }
    const dir = join(scratch, name);
    mkdirSync(dir);
    const workspace = join(dir, "workspace");
    command("git", ["clone", "--quiet", "--shared", root, workspace], dir);
    // Exercise working changes too, before their coherent commit exists.
    for (const file of command("git", ["diff", "--name-only", "-z", "HEAD"], root)
      .split("\0")
      .filter(Boolean)) {
      assert.ok(!file.startsWith("/") && !file.split("/").includes(".."));
      const destination = join(workspace, file);
      mkdirSync(dirname(destination), { recursive: true });
      if (existsSync(join(root, file))) copyFileSync(join(root, file), destination);
      else rmSync(destination, { force: true });
    }
    cpSync(join(root, "node_modules"), join(workspace, "node_modules"), {
      recursive: true,
      verbatimSymlinks: true,
    });
    cpSync(join(root, ".github/node_modules"), join(workspace, ".github/node_modules"), {
      recursive: true,
      verbatimSymlinks: true,
    });
    writeFileSync(
      join(workspace, "fixture-environment-probe.mjs"),
      `
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { fixtureEnvironment } from './.github/fixture-environment.mjs';
assert.equal(process.env.NX_SELF_HOSTED_REMOTE_CACHE_ACCESS_TOKEN, 'outer-fixture-token');
const result = spawnSync(process.execPath, ['node_modules/nx/dist/bin/nx.js', 'run', 'workspace:environment-probe'], {
  stdio: 'inherit', env: fixtureEnvironment({ NX_INVOCATION_ROOT_PID: undefined, ${name === "node-owned-recording" ? `GITHUB_ACTIONS: 'true', RUNNER_TEMP: '.nx/owned-fixture'` : ""} }),
});
assert.equal(result.status, 0);
${name === "node-owned-recording" ? `assert.ok(existsSync('.nx/owned-fixture/nx-cache-recording/outer/invocations.jsonl'));` : ""}
`,
    );
    writeFileSync(
      join(workspace, "fixture-entry-probe.test.mjs"),
      `
import './.github/test-setup.mjs';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
writeFileSync('fixture-entry-location.json', JSON.stringify({temp:process.env.RUNNER_TEMP}));
assert.equal(process.env.NX_SELF_HOSTED_REMOTE_CACHE_SERVER, undefined);
assert.equal(process.env.NX_SELF_HOSTED_REMOTE_CACHE_ACCESS_TOKEN, undefined);
const result = spawnSync(process.execPath, ['node_modules/nx/dist/bin/nx.js', 'run', 'workspace:environment-probe'], {stdio: 'inherit'});
assert.equal(result.status, 0);
`,
    );
    writeFileSync(
      join(workspace, "fixture-shell-entry.sh"),
      `
set -eu
for key in GITHUB_ACTIONS NX_SELF_HOSTED_REMOTE_CACHE_SERVER NX_SELF_HOSTED_REMOTE_CACHE_ACCESS_TOKEN; do
  value="$(printenv "$key" || true)"
  test -z "$value"
done
node node_modules/nx/dist/bin/nx.js run workspace:environment-probe
`,
    );
    writeFileSync(
      join(workspace, "fixture-python-entry.py"),
      `
import os, subprocess
for key in ['GITHUB_ACTIONS','NX_SELF_HOSTED_REMOTE_CACHE_SERVER','NX_SELF_HOSTED_REMOTE_CACHE_ACCESS_TOKEN']:
    assert not os.environ.get(key), key
subprocess.run(['node','node_modules/nx/dist/bin/nx.js','run','workspace:environment-probe'],check=True)
`,
    );
    writeFileSync(
      join(workspace, "fixture-shell-clone.sh"),
      `
set -eu
source scripts/scratch-clone.sh
scratch_clone "$PWD" "$PWD/../clone"
cp -a node_modules "$PWD/../clone/node_modules"
unset NX_INVOCATION_ROOT_PID
export NX_DAEMON=false
cd "$PWD/../clone"
node node_modules/nx/dist/bin/nx.js run workspace:environment-probe
test -s "$RUNNER_TEMP/nx-cache-recording/outer/invocations.jsonl"
`,
    );
    const projectPath = join(workspace, "workspace/project.json");
    const project = JSON.parse(readFileSync(projectPath, "utf8"));
    project.targets["fixture-isolation"] = {
      executor: "nx:run-commands",
      cache: false,
      options: {
        command: tierCommand,
        ...(["shell-entry", "python-entry"].includes(name)
          ? {
              env: JSON.parse(
                readFileSync(
                  join(
                    root,
                    name === "shell-entry" ? "scripts/project.json" : "workspace/project.json",
                  ),
                  "utf8",
                ),
              ).targets.test.options.env,
            }
          : {}),
      },
    };
    project.targets["environment-probe"] = {
      executor: "nx:run-commands",
      cache: false,
      options: {
        command: `node -e 'for (const key of ${JSON.stringify(["node-owned-recording", "shell-clone"].includes(name) ? ["NX_SELF_HOSTED_REMOTE_CACHE_SERVER", "NX_SELF_HOSTED_REMOTE_CACHE_ACCESS_TOKEN"] : ["GITHUB_ACTIONS", "NX_SELF_HOSTED_REMOTE_CACHE_SERVER", "NX_SELF_HOSTED_REMOTE_CACHE_ACCESS_TOKEN"])}) if (process.env[key]) throw Error(key + " leaked")'`,
      },
    };
    writeFileSync(projectPath, JSON.stringify(project));
    command("git", ["add", "-A"], workspace);
    command(
      "git",
      [
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "user.name=fixture",
        "-c",
        "user.email=fixture@invalid",
        "commit",
        "--quiet",
        "-m",
        "test: nested recording fixture",
      ],
      workspace,
    );
    const runnerTemp = join(dir, "runner-temp");
    const result = spawnSync(
      process.execPath,
      ["node_modules/nx/dist/bin/nx.js", "run", "workspace:fixture-isolation"],
      {
        cwd: workspace,
        encoding: "utf8",
        maxBuffer: 64 * 1024 * 1024,
        timeout: 600_000,
        env: fixtureEnvironment({
          NODE_TEST_CONTEXT: undefined,
          GIT_DIR: undefined,
          GIT_WORK_TREE: undefined,
          GIT_INDEX_FILE: undefined,
          GIT_COMMON_DIR: undefined,
          GITHUB_ACTIONS: "true",
          GITHUB_JOB: "outer",
          RUNNER_TEMP: runnerTemp,
          NX_DAEMON: "false",
          NX_INVOCATION_ROOT_PID: undefined,
          NX_SELF_HOSTED_REMOTE_CACHE_SERVER: "http://127.0.0.1:9",
          NX_SELF_HOSTED_REMOTE_CACHE_ACCESS_TOKEN: "outer-fixture-token",
        }),
      },
    );
    const recording = join(runnerTemp, "nx-cache-recording/outer");
    const rows = existsSync(join(recording, "invocations.jsonl"))
      ? readFileSync(join(recording, "invocations.jsonl"), "utf8")
          .trim()
          .split("\n")
          .map((line) => JSON.parse(line))
      : [];
    const tasks = rows.flatMap((row) => row.tasks.map((task) => task.task));
    t.diagnostic(
      `${name}: ${rows.length} invocation(s), ${tasks.length} outer recording row(s); child status ${result.status}`,
    );
    assert.equal(result.status, 0, result.stdout + result.stderr);
    if (name === "node-entry") {
      const { temp } = JSON.parse(
        readFileSync(join(workspace, "fixture-entry-location.json"), "utf8"),
      );
      assert.equal(typeof temp, "string");
      assert.equal(
        dirname(temp),
        process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir(),
        "setup allocates beneath the selected scratch root",
      );
      assert.notEqual(temp, runnerTemp, "setup must replace the inherited recording location");
      assert.equal(
        existsSync(temp),
        false,
        "the private directory is removed when the test process exits",
      );
    }
    assert.deepEqual(
      tasks,
      ["workspace:fixture-isolation"],
      "only the outer task belongs in the job's recording",
    );
    assert.equal(
      readFileSync(join(recording, "started.jsonl"), "utf8").trim().split("\n").length,
      1,
    );
    rmSync(dir, { recursive: true, force: true });
  });
}
