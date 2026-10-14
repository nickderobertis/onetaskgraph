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
test("actual release declarations retain every released platform, package and independent crate tag", () => {
  const release = workflows()["release.yml"];
  const native = release.jobs["native-assets-carriers-and-wheels"];
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
    assert.ok(needs.has("native-assets-carriers-and-wheels"), id);
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
  // Intel macOS is discontinued: the one Mac job builds, packs and proves arm64 natively,
  // with no x86_64 cross-compilation or Rosetta step, and nothing declares the old target.
  assert.equal(mac.env.TARGETS, "aarch64-apple-darwin=darwin-arm64");
  assert.ok(mac.steps.some((s) => s.name === "Install and run the Apple Silicon products"));
  for (const source of [
    read(".github/workflows/release.yml"),
    read("release-targets.toml"),
    read("npm/cli/package.json"),
    read("scripts/product_versions.py"),
  ])
    assert.doesNotMatch(source, /x86_64-apple-darwin|darwin-x64|rosetta/i);
  assert.ok(!carriers.includes("darwin-x64"));
  assert.match(
    read("scripts/install.sh"),
    /Darwin:x86_64\) die 69 "Intel macOS is no longer supported/,
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

test("telemetry records the Apple Silicon target alone on the single runner row", () => {
  const release = workflows()["release.yml"];
  const row = graph({ event: "release", action: "published" }, { macosRequired: false })
    .find((run) => run.workflow === "release.yml")
    .rows.find((row) => row.job === "macos-assets-carriers-and-wheels");
  const declared = release.jobs["macos-assets-carriers-and-wheels"].env.TARGETS.split(" ").map(
    (pair) => pair.split("=")[0],
  );
  assert.deepEqual(row.target.split(",").sort(), declared.sort());
  assert.deepEqual(declared, ["aarch64-apple-darwin"]);
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
        # macOS hands out /var/folders, which the resolver reads as /private/var.
        root = Path(temporary).resolve()
        (root / 'Git/cmd').mkdir(parents=True)
        (root / 'Git/bin').mkdir()
        (root / 'launchers').mkdir()
        wsl = root / 'launchers/bash'
        wsl.write_text('#!/bin/sh\\n echo unexpected-WSL >&2\\nexit 91\\n')
        wsl.chmod(0o755)
        selected = root / 'Git/bin/bash.exe'
        shutil.copy2(actual_bash, selected)
        for directory in ['cmd', 'bin', 'usr/bin', 'mingw64/bin', 'mingw32/bin']:
            git = root / 'Git' / directory / 'git'
            git.parent.mkdir(parents=True, exist_ok=True)
            git.write_text('#!/bin/sh\\nexit 0\\n')
            git.chmod(0o755)
            os.environ['PATH'] = str(wsl.parent) + os.pathsep + str(git.parent)
            resolved = bash_executable('Windows')
            assert resolved == str(selected), directory
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

test("wheel extraction reports corrupt archives and destination IO failures through its CLI", () => {
  execFileSync(
    "python3",
    [
      "-c",
      `import subprocess, sys, tempfile, zipfile
from pathlib import Path
with tempfile.TemporaryDirectory() as temporary:
    tree = Path(temporary) / 'dist/wheels/aarch64-apple-darwin'
    tree.mkdir(parents=True)
    wheel = tree / 'cli.whl'
    binary = Path(temporary) / 'target/aarch64-apple-darwin/release/onetaskgraph'
    binary.parent.parent.mkdir(parents=True)
    command = [sys.executable, 'scripts/release-wheel-binary.py', '--wheels', str(tree), '--binary', str(binary)]
    wheel.write_bytes(b'corrupt ZIP')
    rejected = subprocess.run(command, capture_output=True, text=True)
    assert rejected.returncode == 1
    assert 'wheel extraction failed:' in rejected.stderr and 'next:' in rejected.stderr
    assert not binary.exists()
    with zipfile.ZipFile(wheel, 'w') as archive:
        archive.writestr('cli.data/scripts/onetaskgraph', b'payload')
    binary.parent.write_text('parent is a file')
    rejected = subprocess.run(command, capture_output=True, text=True)
    assert rejected.returncode == 1
    assert 'wheel extraction failed:' in rejected.stderr and 'next:' in rejected.stderr
    binary.parent.unlink()
    installed = subprocess.run(command, capture_output=True, text=True)
    assert installed.returncode == 0, installed.stderr
    assert binary.read_bytes() == b'payload'
`,
    ],
    { cwd: root, stdio: "pipe" },
  );
});

test("wheel extraction refuses paths outside the release layout before writing", () => {
  execFileSync(
    "python3",
    [
      "-c",
      `import subprocess, sys, tempfile, zipfile
from pathlib import Path
with tempfile.TemporaryDirectory() as temporary:
    root = Path(temporary)
    wheels = root / 'dist/wheels/aarch64-apple-darwin'
    wheels.mkdir(parents=True)
    with zipfile.ZipFile(wheels / 'cli.whl', 'w') as archive:
        archive.writestr('cli.data/scripts/onetaskgraph', b'payload')
    stray = root / 'elsewhere/onetaskgraph'
    for wheel_dir, binary, flag in [
        (root / 'dist', root / 'target/aarch64-apple-darwin/release/onetaskgraph', '--wheels'),
        (wheels, stray, '--binary'),
        (wheels, root / 'target/x86_64-unknown-linux-gnu/release/onetaskgraph', '--binary'),
    ]:
        rejected = subprocess.run([sys.executable, 'scripts/release-wheel-binary.py', '--wheels', str(wheel_dir), '--binary', str(binary)], capture_output=True, text=True)
        assert rejected.returncode == 2, rejected.stderr
        assert flag + ' must be' in rejected.stderr and 'next:' in rejected.stderr, rejected.stderr
        assert not binary.exists()
`,
    ],
    { cwd: root, stdio: "pipe" },
  );
});

test("packaging guard rejects build-backend requirement drift before staging products", () => {
  execFileSync(
    "python3",
    [
      "-c",
      `import platform, shutil, subprocess, sys, tempfile
from pathlib import Path
sys.path.insert(0, 'scripts')
from release_products import bash_executable
# A bare 'bash' from Python on Windows is System32's WSL launcher, not Git Bash.
bash = bash_executable(platform.system())
with tempfile.TemporaryDirectory() as temporary:
    tree = Path(temporary)
    shutil.copytree('scripts', tree / 'scripts')
    manifest = Path('pyproject.toml').read_text(encoding='utf-8')
    (tree / 'pyproject.toml').write_text(manifest.replace('maturin>=1.9,<2', 'maturin>=1.10,<2'), encoding='utf-8')
    rejected = subprocess.run([bash, 'scripts/check-release-packaging.sh'], cwd=tree, capture_output=True, text=True, encoding='utf-8')
    assert rejected.returncode == 2, rejected.stderr
    assert 'distribution maturin requirement disagrees with pyproject.toml' in rejected.stderr
    assert 'next:' in rejected.stderr
    assert not (tree / 'dist').exists()
`,
    ],
    { cwd: root, stdio: "pipe" },
  );
});
