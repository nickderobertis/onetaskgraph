import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readdirSync } from "node:fs";
import { test } from "node:test";
import { graph, matrices, read, resolve, root, workflows } from "../workflow-graph.mjs";

function ancestors(jobs, id, seen = new Set()) {
  const needs = typeof jobs[id].needs === "string" ? [jobs[id].needs] : jobs[id].needs || [];
  for (const parent of needs) {
    if (!seen.has(parent)) {
      seen.add(parent);
      ancestors(jobs, parent, seen);
    }
  }
  return seen;
}
test("actual release declarations retain every platform, package and independent crate tag", () => {
  const release = workflows()["release.yml"];
  const native = release.jobs["native-assets-and-carriers"];
  const mac = release.jobs["macos-assets-carriers-and-wheels"];
  const targets = [
    ...matrices(native.strategy.matrix, {}),
    ...mac.env.TARGETS.split(" ").map((pair) => {
      const [target, npm] = pair.split("=");
      return { target, npm, os: mac["runs-on"], ext: "tar.gz" };
    }),
  ];
  const carriers = readdirSync(new URL("npm/platforms/", root));
  assert.deepEqual(targets.map((t) => t.npm).sort(), carriers.sort());
  for (const target of targets) {
    const manifest = JSON.parse(read(`npm/platforms/${target.npm}/package.json`));
    assert.equal(
      manifest.os[0],
      target.target.endsWith("apple-darwin")
        ? "darwin"
        : target.target.endsWith("windows-msvc")
          ? "win32"
          : "linux",
    );
    assert.equal(manifest.cpu[0], target.target.startsWith("aarch64") ? "arm64" : "x64");
    assert.match(
      read("scripts/install.sh"),
      new RegExp(`${target.target}; ext=${target.ext.replace(".", "\\.")}`),
    );
    const job = target.os === "macos-latest" ? mac : native;
    const context = { matrix: target };
    assert.ok(
      job.steps.some(
        (s) =>
          s.uses === "PyO3/maturin-action@v1" && resolve(s.with.target, context) === target.target,
      ),
    );
    assert.ok(
      job.steps.some(
        (s) =>
          s.uses === "actions/upload-artifact@v4" &&
          resolve(s.with.name, context) === `wheels-${target.target}`,
      ),
    );
    assert.ok(
      job.steps.some(
        (s) =>
          s.uses === "actions/upload-artifact@v4" &&
          resolve(s.with.name, context) === `carrier-${target.npm}`,
      ),
    );
  }
  for (const [id, job] of Object.entries(release.jobs).filter(([id]) =>
    id.startsWith("publish-"),
  )) {
    assert.equal(job["runs-on"], "ubuntu-latest", id);
    const needs = ancestors(release.jobs, id);
    assert.ok(needs.has("macos-assets-carriers-and-wheels"), id);
    assert.ok(needs.has("native-assets-and-carriers"), id);
  }
  for (const id of ["publish-python", "publish-npm", "publish-crates"])
    assert.ok(ancestors(release.jobs, id).has("publish-assets"), id);
  for (const job of [mac, native]) {
    const wheel = job.steps.findIndex((s) => s.uses === "PyO3/maturin-action@v1");
    const extract = job.steps.findIndex((s) => s.run?.includes("scripts/release-wheel-binary.py"));
    const pack = job.steps.findIndex((s) => s.name === "Archive, checksum, and stage");
    const install = job.steps.findIndex(
      (s) => s.run?.includes("scripts/verify-release-products.py") && s.run.includes("--install"),
    );
    const upload = job.steps.findIndex((s) => s.uses === "actions/upload-artifact@v4");
    assert.ok(
      wheel >= 0 && wheel < extract && extract < pack && pack < install && install < upload,
    );
    assert.ok(!job.steps.some((s) => s.run?.includes("gh release upload")));
    assert.ok(
      job.steps.some((s) => s.uses === "actions/checkout@v7" && !s.with?.ref),
      "build consumes the release event exact tag",
    );
  }
  assert.equal(mac["runs-on"], "macos-latest");
  assert.match(
    mac.steps.find((s) => s.name === "Install and run both Darwin architectures").run,
    /install-rosetta/,
  );
  const config = JSON.parse(
    execFileSync(
      "python3",
      [
        "-c",
        'import tomllib,json; print(json.dumps(tomllib.load(open("release-targets.toml","rb"))))',
      ],
      { cwd: root, encoding: "utf8" },
    ),
  );
  const ids = config.target.flatMap((t) => [t.id, ...(t.covers || [])]);
  for (const carrier of carriers) assert.ok(ids.includes(`npm:@onetaskgraph/cli-${carrier}`));
  for (const id of [
    "pypi:onetaskgraph-cli",
    "pypi:onetaskgraph-sdk",
    "npm:@onetaskgraph/cli",
    "npm:@onetaskgraph/sdk",
  ])
    assert.ok(ids.includes(id));
  const releasePlz = JSON.parse(
    execFileSync(
      "python3",
      ["-c", 'import tomllib,json; print(json.dumps(tomllib.load(open("release-plz.toml","rb"))))'],
      { cwd: root, encoding: "utf8" },
    ),
  );
  for (const p of releasePlz.package) {
    assert.equal(p.git_tag_name, `${p.name}-v{{ version }}`);
    assert.equal(p.git_release_enable, false);
  }
  assert.equal(releasePlz.workspace.git_tag_name, "v{{ version }}");
});

test("release routing prefix agrees with the production PR builder", () => {
  assert.match(read("scripts/prepare-release-pr.sh"), /branch="release-plz-\$version"/);
  assert.match(read(".github/macos-policy.mjs"), /startsWith\("release-plz-"\)/);
});

test("Mac comparison base is the exact production body and environment exercised by the live-lane journey", () => {
  const ci = workflows()["ci.yml"];
  const original = ci.jobs.check.steps.find((step) => step.id === "base");
  const combined = ci.jobs.macos.steps.find((step) => step.id === "macos-base");
  // scripts/check-live-lane-selection.sh executes original.run over merge-base,
  // predecessor, HEAD~1 and missing-base histories; equality extends that same proof.
  assert.equal(combined.run, original.run);
  assert.deepEqual(combined.env, original.env);
  assert.match(read("justfile"), /^check:.*distribution-test/m);
  assert.ok(ci.jobs.macos.steps.some((step) => step.run === "just check"));
  assert.ok(!ci.jobs.macos.steps.some((step) => step.run === "just distribution-test"));
});

test("telemetry retains both compiled Darwin architectures on the single runner row", () => {
  const release = workflows()["release.yml"];
  const row = graph({ event: "release", action: "published" }, { macosRequired: false })
    .find((run) => run.workflow === "release.yml")
    .rows.find((row) => row.job === "macos-assets-carriers-and-wheels");
  const declared = release.jobs["macos-assets-carriers-and-wheels"].env.TARGETS.split(" ").map(
    (pair) => pair.split("=")[0],
  );
  assert.deepEqual(row.target.split(",").sort(), declared.sort());
});

test("release subprocesses choose Git Bash over a WSL launcher and refuse missing Git Bash", () => {
  execFileSync(
    "python3",
    [
      "-c",
      `import os, platform, shutil, subprocess, sys, tempfile
from pathlib import Path
sys.path.insert(0, 'scripts')
from release_products import bash_executable
if platform.system() == 'Windows':
    subprocess.run([bash_executable('Windows'), '-c', 'exit 0'], check=True)
else:
    actual_bash = shutil.which('bash')
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        (root / 'Git/cmd').mkdir(parents=True)
        (root / 'Git/bin').mkdir()
        (root / 'launchers').mkdir()
        git = root / 'Git/cmd/git'
        git.write_text('#!/bin/sh\\nexit 0\\n')
        git.chmod(0o755)
        wsl = root / 'launchers/bash'
        wsl.write_text('#!/bin/sh\\necho unexpected-WSL >&2\\nexit 91\\n')
        wsl.chmod(0o755)
        selected = root / 'Git/bin/bash.exe'
        shutil.copy2(actual_bash, selected)
        os.environ['PATH'] = str(wsl.parent) + os.pathsep + str(git.parent)
        resolved = bash_executable('Windows')
        assert resolved == str(selected)
        subprocess.run([resolved, '-c', 'printf Git-Bash-selected'], check=True)
        assert subprocess.run([resolved, '-c', 'exit 37'], check=False).returncode == 37
        selected.unlink()
        try:
            bash_executable('Windows')
        except ValueError as error:
            assert 'install Git for Windows' in str(error)
        else:
            raise AssertionError('missing Git Bash fell back to WSL')
`,
    ],
    { cwd: root, stdio: "pipe" },
  );
});

test("workflow tooling format and lint commands run without shell glob expansion", () => {
  const project = JSON.parse(read(".github/project.json"));
  for (const name of ["format-check", "lint"]) {
    const options = project.targets[name].options;
    for (const command of options.commands || [options.command]) {
      assert.match(command, /^bun run biome [a-z .*/-]+$/);
      execFileSync("bun", command.split(" ").slice(1), { cwd: root, stdio: "pipe" });
    }
  }
});
