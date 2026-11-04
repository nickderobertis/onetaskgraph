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
  ["affected-selection", "bash scripts/check-affected-selection.sh"],
  ["ci-recipes", "bash scripts/check-ci-recipes.sh"],
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
    if (name === "affected-selection" || name === "ci-recipes") {
      // These guards already double fixture task commands. Keep their real Nx
      // and job boundaries and make every successful task probe credential removal.
      const path = join(workspace, `scripts/check-${name}.sh`);
      const original = readFileSync(path, "utf8");
      const site =
        name === "affected-selection"
          ? '            target["options"] = {"command": "exit 1" if failing else command}'
          : '        definition["options"] = {"command": "exit 1" if task == failing else "true"}';
      assert.ok(
        original.includes(site),
        "environment probe must reach the real guard's fixture commands",
      );
      const probe =
        'python3 -c \'import os; assert not os.environ.get("NX_SELF_HOSTED_REMOTE_CACHE_SERVER"); assert not os.environ.get("NX_SELF_HOSTED_REMOTE_CACHE_ACCESS_TOKEN")\'';
      const replacement =
        name === "affected-selection"
          ? `            command = ${JSON.stringify(probe)} + " && " + command\n${site}`
          : `        definition["options"] = {"command": "exit 1" if task == failing else ${JSON.stringify(probe)}}`;
      writeFileSync(path, original.replace(site, replacement));
    }
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
    const projectPath = join(workspace, "workspace/project.json");
    const project = JSON.parse(readFileSync(projectPath, "utf8"));
    project.targets["fixture-isolation"] = {
      executor: "nx:run-commands",
      cache: false,
      options: { command: tierCommand },
    };
    project.targets["environment-probe"] = {
      executor: "nx:run-commands",
      cache: false,
      options: {
        command: `node -e 'for (const key of ${JSON.stringify(name === "node-owned-recording" ? ["NX_SELF_HOSTED_REMOTE_CACHE_SERVER", "NX_SELF_HOSTED_REMOTE_CACHE_ACCESS_TOKEN"] : ["GITHUB_ACTIONS", "NX_SELF_HOSTED_REMOTE_CACHE_SERVER", "NX_SELF_HOSTED_REMOTE_CACHE_ACCESS_TOKEN"])}) if (process.env[key]) throw Error(key + " leaked")'`,
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
