import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readdirSync, rmSync, writeFileSync } from "node:fs";
import { test } from "node:test";
import { digest, graph, read, root, workflows } from "../workflow-graph.mjs";

const RELEASE_JOBS = {
  "build-linux": { runner: "ubuntu-latest", recipe: "linux", platform: "linux" },
  "build-linux-arm64": {
    runner: "ubuntu-24.04-arm",
    recipe: "linux-arm64",
    platform: "linux-arm64",
  },
  "build-macos": { runner: "macos-latest", recipe: "macos", platform: "macos" },
  "build-windows": { runner: "windows-latest", recipe: "windows", platform: "windows" },
  publish: { runner: "ubuntu-latest", recipe: "publish", platform: "independent" },
};
const PLATFORM_OF = {
  linux: "linux",
  "linux-arm64": "linux-arm64",
  macos: "macos",
  windows: "windows",
  publish: "independent",
};
const CONTROL_OF = { pypi: "PYPI_PUBLISH", npm: "NPM_PUBLISH", crates: "CARGO_PUBLISH" };
const version = JSON.parse(read("npm/cli/package.json")).version;
const plans = new Map();
function plan(platform, tag = `v${version}`) {
  const key = `${platform}@${tag}`;
  if (!plans.has(key))
    plans.set(
      key,
      JSON.parse(
        execFileSync(
          "python3",
          ["scripts/release_cd.py", "--platform", platform, "plan", "--tag", tag],
          { cwd: root, encoding: "utf8" },
        ),
      ),
    );
  return plans.get(key);
}
// One step is one phase of one recipe, or it is setup; nothing else runs in a CD job.
function phase(step) {
  const match =
    /^just release-(linux-arm64|linux|macos|windows|publish) (plan|build|publish|verify|available)(?: --(?:to|from) (github|pypi|npm|crates))? --tag "\$TAG"$/.exec(
      (step.run || "").trim(),
    );
  return match && { recipe: match[1], phase: match[2], registry: match[3] };
}
function tomlDocument(path) {
  return JSON.parse(
    execFileSync(
      "python3",
      [
        "-c",
        "import json, sys, tomllib; print(json.dumps(tomllib.load(open(sys.argv[1], 'rb'))))",
        path,
      ],
      { cwd: root, encoding: "utf8" },
    ),
  );
}
function uploads() {
  const release = workflows()["release.yml"];
  const found = [];
  for (const [job, definition] of Object.entries(release.jobs))
    definition.steps.forEach((step, index) => {
      const parsed = phase(step);
      if (parsed?.phase !== "publish") return;
      const artifacts = plan(PLATFORM_OF[parsed.recipe]).filter(
        (artifact) => artifact.registry === parsed.registry,
      );
      found.push({ job, index, step, registry: parsed.registry, artifacts });
    });
  return found;
}

test("release.yml is five jobs on the runners their names say, and release-plz.yml is one", () => {
  const all = workflows();
  const release = all["release.yml"];
  assert.deepEqual(Object.keys(release.jobs).sort(), Object.keys(RELEASE_JOBS).sort());
  for (const [id, expected] of Object.entries(RELEASE_JOBS)) {
    const job = release.jobs[id];
    assert.equal(job["runs-on"], expected.runner, id);
    assert.equal(job["timeout-minutes"], 30, id);
    for (const step of job.steps.filter((step) => step.run !== undefined)) {
      const parsed = phase(step);
      assert.ok(parsed, `${id} runs '${step.run}', which is not a phase of its recipe`);
      assert.equal(parsed.recipe, expected.recipe, `${id} runs another job's recipe`);
    }
  }
  for (const id of ["build-linux", "build-linux-arm64", "build-macos", "build-windows"])
    assert.equal(release.jobs[id].needs, undefined, `${id} waits on another build`);
  assert.deepEqual([...release.jobs.publish.needs].sort(), [
    "build-linux",
    "build-linux-arm64",
    "build-macos",
    "build-windows",
  ]);
  assert.deepEqual(Object.keys(all["release-plz.yml"].jobs), ["release-plz"]);
  assert.ok(!Object.keys(all).includes("published-smoke.yml"));
  for (const [name, workflow] of Object.entries(all)) {
    const downstream = typeof workflow.on === "object" && workflow.on?.workflow_run;
    assert.ok(
      !downstream || !(downstream.workflows || []).includes("Release"),
      `${name} triggers on the Release workflow`,
    );
  }
  // CI already proved the tree; a CD job re-running its checks or test tiers proves it again.
  for (const name of ["release.yml", "release-plz.yml"])
    for (const [id, job] of Object.entries(all[name].jobs))
      for (const step of job.steps)
        assert.doesNotMatch(
          step.run || "",
          /\bjust (check|gate|test|coverage|distribution-check|distribution-test)\b|nx(\.sh)? (affected|run-many)|cargo (test|nextest|llvm-cov)/,
          `${name}/${id} re-runs a check tier`,
        );
});

test("every artifact is uploaded by exactly one step of the job release-targets.toml names", () => {
  const sent = uploads();
  const owners = new Map();
  for (const upload of sent) {
    assert.ok(upload.artifacts.length, `${upload.job} publishes nothing to ${upload.registry}`);
    for (const artifact of upload.artifacts) {
      assert.equal(artifact.job, upload.job, `${artifact.file} is published by the wrong job`);
      owners.set(artifact.file, [...(owners.get(artifact.file) || []), upload]);
    }
  }
  const everything = Object.values(RELEASE_JOBS).flatMap(({ platform }) => plan(platform));
  assert.ok(everything.length);
  for (const artifact of everything)
    assert.equal(owners.get(artifact.file)?.length, 1, `${artifact.file} is not uploaded once`);
  const jobsOf = (ids) =>
    new Set(
      sent.filter((u) => u.artifacts.some((a) => ids.includes(a.release_target))).map((u) => u.job),
    );
  for (const target of tomlDocument("release-targets.toml").target) {
    const ids = [target.id, ...(target.covers || [])];
    for (const id of ids)
      assert.ok(
        everything.some((artifact) => artifact.release_target === id),
        `release-targets.toml declares ${id}, which no job uploads`,
      );
    const named = new Set(
      [
        ...target.published_by.matchAll(
          /((?:(?:build-[a-z]+(?:-arm64)?|publish)(?:, and |, | and ))*(?:build-[a-z]+(?:-arm64)?|publish)) jobs?\b/g,
        ),
      ].flatMap((match) => match[1].split(/, and |, | and /)),
    );
    assert.deepEqual([...named].sort(), [...jobsOf(ids)].sort(), `${target.id}'s published_by`);
  }
  // The publish job carries what is the same on every platform, and nothing that is not.
  for (const upload of sent.filter((u) => u.job === "publish"))
    for (const artifact of upload.artifacts) {
      assert.equal(artifact.platform, "independent", artifact.file);
      assert.equal(artifact.target, undefined, artifact.file);
    }
});

test("each build job test-installs before it publishes and installs back from every registry after", () => {
  const release = workflows()["release.yml"];
  for (const [id, { recipe }] of Object.entries(RELEASE_JOBS)) {
    const steps = release.jobs[id].steps.map((step, index) => ({ step, index, ...phase(step) }));
    const of = (name) => steps.filter((s) => s.phase === name);
    const [build] = of("build");
    assert.ok(build, `${id} has no build step`);
    assert.equal(build.step.if, undefined, `${id} builds conditionally`);
    const publications = of("publish");
    const verifications = of("verify");
    assert.ok(publications.length && verifications.length, id);
    const firstPublication = Math.min(...publications.map((s) => s.index));
    assert.ok(build.index < firstPublication, `${id} publishes before it test-installs`);
    assert.ok(
      Math.max(...publications.map((s) => s.index)) <
        Math.min(...verifications.map((s) => s.index)),
      `${id} verifies a registry before it has published everything`,
    );
    assert.deepEqual(
      verifications.map((s) => s.registry).sort(),
      publications.map((s) => s.registry).sort(),
      `${id} does not verify every registry it publishes to`,
    );
    // A registry the repository has not switched on is neither published to nor read back.
    for (const s of [...publications, ...verifications, ...of("available")]) {
      const control = CONTROL_OF[s.registry];
      assert.equal(
        s.step.if,
        control ? `\${{ vars.${control} == 'true' }}` : undefined,
        `${id}: ${s.step.name}`,
      );
    }
    if (recipe === "publish") {
      const available = of("available");
      assert.deepEqual(available.map((s) => s.registry).sort(), ["github", "npm", "pypi"]);
      assert.ok(Math.max(...available.map((s) => s.index)) < firstPublication);
    }
  }
});

test("Intel macOS is neither built, published, verified nor declared, and Linux arm64 is", () => {
  const sources = [
    ...Object.keys(workflows()).map((name) => read(`.github/workflows/${name}`)),
    read("justfile"),
    read("release-targets.toml"),
    read("npm/cli/package.json"),
    read("scripts/release_products.py"),
    read("scripts/release_cd.py"),
    JSON.stringify(Object.values(RELEASE_JOBS).flatMap(({ platform }) => plan(platform))),
  ];
  for (const source of sources)
    assert.doesNotMatch(source, /x86_64-apple-darwin|darwin-x64|rosetta/i);
  const carriers = readdirSync(new URL("npm/platforms/", root)).sort();
  assert.deepEqual(carriers, ["darwin-arm64", "linux-arm64", "linux-x64", "win32-x64"]);
  // Linux arm64 is built natively on GitHub's hosted ARM runner, never cross-compiled.
  const arm = workflows()["release.yml"].jobs["build-linux-arm64"];
  assert.equal(arm["runs-on"], "ubuntu-24.04-arm");
  assert.ok(
    arm.steps.some((step) => step.with?.target === "aarch64-unknown-linux-gnu"),
    "build-linux-arm64 installs the aarch64-unknown-linux-gnu toolchain",
  );
  assert.deepEqual(
    plan("linux-arm64")
      .map((artifact) => artifact.target)
      .filter((target, index, all) => all.indexOf(target) === index),
    ["aarch64-unknown-linux-gnu"],
  );
  assert.match(
    read("scripts/install.sh"),
    /Linux:aarch64\|Linux:arm64\) target=aarch64-unknown-linux-gnu; ext=tar\.gz;/,
  );
  assert.match(
    read("scripts/install.sh"),
    /Darwin:x86_64\) die 69 "Intel macOS is no longer supported/,
  );
  const releasePlz = tomlDocument("release-plz.toml");
  for (const p of releasePlz.package) {
    assert.equal(p.git_tag_name, `${p.name}-v{{ version }}`);
    assert.equal(p.git_release_enable, false);
  }
  assert.equal(releasePlz.workspace.git_tag_name, "v{{ version }}");
});

// Plan mode, not a build: The macOS and Windows build recipes need native macOS and Windows toolchains this Linux host cannot run, so inside the dispatch their proof is the recipes' plan mode checked against the last release's published artifacts; the Linux recipe is driven for real end to end, and the hosted build jobs build and run the foreign artifacts on every release.
test("each platform's plan is what v0.7.1 published for its targets, at any version", () => {
  const published = JSON.parse(read(".github/tests/fixtures/v0.7.1-published.json"));
  const retired = ["x86_64-apple-darwin"];
  const expected = (golden, from, to) => JSON.parse(JSON.stringify(golden).replaceAll(from, to));
  for (const releaseVersion of [published.version, "9.8.7"]) {
    const golden = expected(published, published.version, releaseVersion);
    const tag = `v${releaseVersion}`;
    const planned = {};
    for (const platform of ["linux", "linux-arm64", "macos", "windows"])
      for (const artifact of plan(platform, tag)) {
        planned[artifact.target] ??= { platform, assets: [], wheel: null, npm: null };
        if (artifact.registry === "github") planned[artifact.target].assets.push(artifact.file);
        if (artifact.kind === "wheel") {
          planned[artifact.target].wheel = artifact.file;
          assert.deepEqual(artifact.wheel_tags, [
            artifact.file
              .split("-")
              .slice(2)
              .join("-")
              .replace(/\.whl$/, ""),
          ]);
          assert.equal(artifact.package, "onetaskgraph-cli");
        }
        if (artifact.kind === "npm-platform") planned[artifact.target].npm = artifact;
      }
    assert.deepEqual(
      Object.keys(planned).sort(),
      Object.keys(golden.targets)
        .filter((target) => !retired.includes(target))
        .sort(),
    );
    for (const [target, row] of Object.entries(planned)) {
      const before = golden.targets[target];
      assert.deepEqual(row.assets.sort(), [...before.assets].sort(), target);
      assert.equal(row.wheel, before.wheel, target);
      assert.equal(row.npm.package, before.npm.name, target);
      // npm pack names a scoped package's tarball <scope>-<name>-<version>.tgz.
      assert.equal(
        row.npm.file,
        `${before.npm.name.slice(1).replace("/", "-")}-${releaseVersion}.tgz`,
      );
      const manifest = JSON.parse(
        read(`npm/platforms/${before.npm.name.split("cli-")[1]}/package.json`),
      );
      assert.deepEqual([manifest.os, manifest.cpu], [before.npm.os, before.npm.cpu], target);
      assert.equal(
        row.platform,
        {
          "aarch64-unknown-linux-gnu": "linux-arm64",
          "aarch64-apple-darwin": "macos",
          "x86_64-pc-windows-msvc": "windows",
        }[target] || "linux",
      );
    }
    const independent = plan("independent", tag);
    assert.deepEqual(
      independent.filter((a) => a.registry === "crates").map((a) => a.package),
      golden.independent.crates,
    );
    assert.deepEqual(
      independent
        .filter((a) => a.registry === "pypi")
        .map((a) => a.file)
        .sort(),
      [...golden.independent.pypi["onetaskgraph-sdk"]].sort(),
    );
    assert.deepEqual(
      independent
        .filter((a) => a.registry === "npm")
        .map((a) => a.package)
        .sort(),
      [...golden.independent.npm].sort(),
    );
  }
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

test("telemetry records build-macos as the one Mac job of a release", () => {
  const rows = graph({ event: "release", action: "published" }, { macosRequired: false })
    .find((run) => run.workflow === "release.yml")
    .rows.filter((row) => row.macos);
  assert.deepEqual(
    rows.map((row) => row.job),
    ["build-macos"],
  );
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
        # A package-manager shim outside the installation answers through git's own exec path.
        shim = root / 'shims/git'
        shim.parent.mkdir()
        shim.write_text('#!/bin/sh\\n[ "$1" = --exec-path ] && echo ' + str(root / 'Git/mingw64/libexec/git-core') + '\\nexit 0\\n')
        shim.chmod(0o755)
        os.environ['PATH'] = str(wsl.parent) + os.pathsep + str(shim.parent)
        assert bash_executable('Windows') == str(selected)
        shim.write_text('#!/bin/sh\\nexit 0\\n')
        try:
            bash_executable('Windows')
        except ValueError as error:
            assert 'install Git for Windows' in str(error)
        else:
            raise AssertionError('a shim naming no installation resolved a bash')
        shim.write_text('#!/bin/sh\\n[ "$1" = --exec-path ] && echo ' + str(root / 'Git/mingw64/libexec/git-core') + '\\nexit 0\\n')
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
    checkout = Path(temporary).resolve()
    tree = checkout / 'dist/wheels/aarch64-apple-darwin'
    tree.mkdir(parents=True)
    wheel = tree / 'cli.whl'
    binary = checkout / 'target/aarch64-apple-darwin/release/onetaskgraph'
    binary.parent.parent.mkdir(parents=True)
    script = str(Path('scripts/release-wheel-binary.py').resolve())
    # A relative interpreter would not resolve from the temporary checkout.
    command = [str(Path(sys.executable).resolve()), script, '--wheels', str(tree), '--binary', str(binary)]
    wheel.write_bytes(b'corrupt ZIP')
    rejected = subprocess.run(command, cwd=checkout, capture_output=True, text=True)
    assert rejected.returncode == 1
    assert 'wheel extraction failed:' in rejected.stderr and 'next:' in rejected.stderr
    assert not binary.exists()
    with zipfile.ZipFile(wheel, 'w') as archive:
        archive.writestr('cli.data/scripts/onetaskgraph', b'payload')
    binary.parent.write_text('parent is a file')
    rejected = subprocess.run(command, cwd=checkout, capture_output=True, text=True)
    assert rejected.returncode == 1
    assert 'wheel extraction failed:' in rejected.stderr and 'next:' in rejected.stderr
    binary.parent.unlink()
    installed = subprocess.run(command, cwd=checkout, capture_output=True, text=True)
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
    root = Path(temporary).resolve()
    outside = root / 'outside'
    outside.mkdir()
    wheels = root / 'dist/wheels/aarch64-apple-darwin'
    wheels.mkdir(parents=True)
    with zipfile.ZipFile(wheels / 'cli.whl', 'w') as archive:
        archive.writestr('cli.data/scripts/onetaskgraph', b'payload')
    stray = root / 'elsewhere/onetaskgraph'
    for wheel_dir, binary, flag in [
        (root / 'dist', root / 'target/aarch64-apple-darwin/release/onetaskgraph', '--wheels'),
        (wheels, stray, '--binary'),
        (wheels, root / 'target/x86_64-unknown-linux-gnu/release/onetaskgraph', '--binary'),
        (wheels, outside / 'target/aarch64-apple-darwin/release/onetaskgraph', '--binary'),
    ]:
        script = str(Path('scripts/release-wheel-binary.py').resolve())
        rejected = subprocess.run([str(Path(sys.executable).resolve()), script, '--wheels', str(wheel_dir), '--binary', str(binary)], cwd=root, capture_output=True, text=True)
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

test("the telemetry digest moves with a nested fixture's content", () => {
  const nested = new URL(`.github/tests/fixtures/digest-probe-${process.pid}.json`, root);
  const without = digest();
  try {
    writeFileSync(nested, "{}\n");
    const added = digest();
    writeFileSync(nested, "[]\n");
    const changed = digest();
    assert.notEqual(added, without);
    assert.notEqual(changed, added);
  } finally {
    rmSync(nested, { force: true });
  }
  assert.equal(digest(), without);
});
