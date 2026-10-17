// The CD jobs' recipes, driven the way the release workflow drives them: the real
// `just release-<job>` recipes, the real maturin, npm, uv, pip and cargo, and
// scripts/loopback-release-registries.py standing in for the GitHub Release, PyPI, npm and
// crates.io. Only the Linux recipe builds here — it is the one this host can build and run —
// and the macOS and Windows publications are driven over stand-in files bearing their planned
// names, which is all a registry reads. Elsewhere these journeys skip, naming the job that
// drives its own platform on every release.
import assert from "node:assert/strict";
import { execFileSync, spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, before, test } from "node:test";
import { fileURLToPath } from "node:url";
import { root } from "../workflow-graph.mjs";

const checkout = fileURLToPath(root);
const skip =
  process.platform === "linux" && process.arch === "x64"
    ? false
    : "the Linux recipe builds and installs x86_64 Linux artifacts natively; build-macos and build-windows drive their own on every release";
const version = JSON.parse(readFileSync(join(checkout, "npm/cli/package.json"), "utf8")).version;
const tag = `v${version}`;
const scratch = mkdtempSync(join(tmpdir(), "release-cd-"));
const dist = join(scratch, "dist");
const log = join(scratch, "commands.log");
let double;
let ports;

function plan(platform, planTag = tag) {
  return JSON.parse(
    execFileSync(
      "python3",
      ["scripts/release_cd.py", "--platform", platform, "plan", "--tag", planTag],
      {
        cwd: checkout,
        encoding: "utf8",
      },
    ),
  );
}
const sha256 = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");

// A stand-in for each tool a publication could go through, recording every invocation and
// handing it to the real one; `extra` lets a journey make one of them fail.
function shims(directory, extra = {}) {
  mkdirSync(directory, { recursive: true });
  for (const name of ["uv", "npm", "cargo", "gh", "curl"]) {
    const real = spawnSync("bash", ["-c", `command -v ${name}`], {
      encoding: "utf8",
    }).stdout.trim();
    if (!real) continue;
    const path = join(directory, name);
    writeFileSync(
      path,
      `#!/bin/sh\nprintf '%s %s\\n' ${name} "$*" >> "${log}"\n${extra[name] || ""}\nexec "${real}" "$@"\n`,
    );
    chmodSync(path, 0o755);
  }
  return directory;
}
function invocations() {
  return existsSync(log) ? readFileSync(log, "utf8").split("\n").filter(Boolean) : [];
}
function recipe(job, args, env = {}) {
  const result = spawnSync("just", [`release-${job}`, ...args], {
    cwd: checkout,
    encoding: "utf8",
    env: { ...process.env, PATH: `${shims(join(scratch, "shims"))}:${process.env.PATH}`, ...env },
    maxBuffer: 1 << 26,
  });
  return { status: result.status, output: `${result.stdout}${result.stderr}` };
}
function expectStatus(result, status, what) {
  assert.equal(result.status, status, `${what}:\n${result.output.slice(-4000)}`);
}

function control(value) {
  writeFileSync(join(scratch, "registries", "control.json"), JSON.stringify(value));
}
function requests() {
  const path = join(scratch, "registries", "requests.jsonl");
  return existsSync(path)
    ? readFileSync(path, "utf8")
        .split("\n")
        .filter(Boolean)
        .map((line) => JSON.parse(line))
    : [];
}
// Every registry address, pointed at the double; unset, each is the real registry.
function registries() {
  const main = `http://127.0.0.1:${ports.main}`;
  return {
    GITHUB_API_URL: `${main}/github/api`,
    GITHUB_REPOSITORY: "nickderobertis/onetaskgraph",
    GH_TOKEN: "fixture-token",
    UV_PUBLISH_TOKEN: "fixture-token",
    ONETASKGRAPH_PYPI_UPLOAD_URL: `${main}/pypi/legacy/`,
    ONETASKGRAPH_PYPI_SIMPLE_URL: `${main}/pypi/simple/`,
    NPM_REGISTRY: `${main}/npm/`,
    NODE_AUTH_TOKEN: "fixture-token",
    npm_config_cache: join(scratch, "npm-cache"),
    ONETASKGRAPH_RELEASE_BASE_URL: `${main}/github/download`,
    ONETASKGRAPH_CHECKSUM_BASE_URL: `http://127.0.0.1:${ports.checksums}/github/download`,
    ONETASKGRAPH_CARGO_REGISTRY: "release-double",
    CARGO_REGISTRIES_RELEASE_DOUBLE_INDEX: `sparse+${main}/crates/index/`,
    CARGO_REGISTRIES_RELEASE_DOUBLE_TOKEN: "fixture-token",
    ONETASKGRAPH_CRATES_API_BASE_URL: `${main}/crates/api/v1/crates`,
    ONETASKGRAPH_CRATES_INDEX_URL: `sparse+${main}/crates/index/`,
    CARGO_REGISTRY_TOKEN: "",
  };
}

before(async () => {
  if (skip) return;
  const state = join(scratch, "registries");
  mkdirSync(state);
  double = spawn("python3", ["scripts/loopback-release-registries.py", state], {
    cwd: checkout,
    stdio: ["ignore", "ignore", "inherit"],
  });
  for (let waited = 0; !existsSync(join(state, "ports.json")); waited += 100) {
    assert.ok(waited < 30000, "the registry double never reported its ports");
    assert.equal(double.exitCode, null, "the registry double exited before it bound");
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  ports = JSON.parse(readFileSync(join(state, "ports.json"), "utf8"));
});
after(() => {
  double?.kill();
  rmSync(scratch, { recursive: true, force: true });
});

test("the Linux recipe builds once, packages every artifact from that binary, installs them and publishes nothing in its dry run", {
  skip,
}, () => {
  const result = recipe("linux", ["run", "--tag", tag, "--dry-run", "--dist", dist]);
  expectStatus(result, 0, "the Linux recipe's dry run");
  const commands = invocations();
  const builds = commands.filter((line) => /^uv tool run .* maturin build /.test(line));
  assert.equal(builds.length, 1, "one target, built once");
  assert.match(builds[0], /--target x86_64-unknown-linux-gnu/);
  assert.ok(
    !commands.some((line) => /^(uv publish|npm publish|cargo publish|gh |curl )/.test(line)),
  );
  const planned = plan("linux");
  for (const artifact of planned)
    assert.match(
      result.output,
      new RegExp(
        `dry run: would publish ${artifact.file.replaceAll(".", "\\.")} to ${artifact.registry}\\b`,
      ),
    );
  // What it built is what v0.7.1 published for this target, at this version.
  const published = JSON.parse(
    readFileSync(join(checkout, ".github/tests/fixtures/v0.7.1-published.json"), "utf8"),
  );
  const golden = JSON.parse(
    JSON.stringify(published.targets["x86_64-unknown-linux-gnu"]).replaceAll("0.7.1", version),
  );
  assert.deepEqual(readdirSync(join(dist, "assets")).sort(), [...golden.assets].sort());
  assert.deepEqual(readdirSync(join(dist, "wheels/x86_64-unknown-linux-gnu")), [golden.wheel]);
  const carrier = `onetaskgraph-cli-linux-x64-${version}.tgz`;
  assert.deepEqual(readdirSync(join(dist, "carriers/linux-x64")), [carrier]);
  // One binary: the bytes inside the archive, the wheel and the npm package are the build's,
  // the wheel is tagged and the binary linked no later than the glibc floor v0.7.1 had.
  const inspected = JSON.parse(
    execFileSync(
      "python3",
      [
        "-c",
        `import hashlib, io, json, re, sys, tarfile, zipfile
assets, wheel, carrier = sys.argv[1:]
digest = lambda data: hashlib.sha256(data).hexdigest()
with tarfile.open(assets) as packed:
    names = packed.getnames()
    archived = packed.extractfile('onetaskgraph').read()
with zipfile.ZipFile(wheel) as packed:
    wheeled = packed.read(next(n for n in packed.namelist() if n.endswith('.data/scripts/onetaskgraph')))
    tags = [line[5:] for line in packed.read(next(n for n in packed.namelist() if n.endswith('.dist-info/WHEEL'))).decode().splitlines() if line.startswith('Tag: ')]
with tarfile.open(carrier) as packed:
    carried = packed.extractfile('package/bin/onetaskgraph').read()
    manifest = json.load(packed.extractfile('package/package.json'))
glibc = max((int(a), int(b)) for a, b in re.findall(rb'GLIBC_(\\d+)\\.(\\d+)', archived))
print(json.dumps({'names': names, 'digests': [digest(archived), digest(wheeled), digest(carried)], 'tags': tags, 'manifest': manifest, 'glibc': list(glibc)}))`,
        join(
          dist,
          "assets",
          golden.assets.find((name) => name.endsWith(".tar.gz")),
        ),
        join(dist, "wheels/x86_64-unknown-linux-gnu", golden.wheel),
        join(dist, "carriers/linux-x64", carrier),
      ],
      { encoding: "utf8" },
    ),
  );
  assert.deepEqual(inspected.names, ["onetaskgraph"]);
  const binary = sha256(join(checkout, "target/x86_64-unknown-linux-gnu/release/onetaskgraph"));
  assert.deepEqual(inspected.digests, [binary, binary, binary]);
  assert.deepEqual(inspected.tags, ["py3-none-manylinux_2_39_x86_64"]);
  assert.ok(inspected.glibc[0] === 2 && inspected.glibc[1] <= 39, `links GLIBC ${inspected.glibc}`);
  assert.equal(inspected.manifest.name, golden.npm.name);
  assert.deepEqual(
    [inspected.manifest.os, inspected.manifest.cpu],
    [golden.npm.os, golden.npm.cpu],
  );
  assert.equal(inspected.manifest.version, version);
});

test("a test-install that fails stops the Linux recipe before any publication", { skip }, () => {
  writeFileSync(log, "");
  const failing = shims(join(scratch, "failing-shims"), {
    npm: 'case "$1" in install) echo "simulated test-install failure" >&2; exit 1;; esac',
  });
  const result = spawnSync(
    "just",
    ["release-linux", "run", "--tag", tag, "--dry-run", "--dist", join(scratch, "dist-failing")],
    {
      cwd: checkout,
      encoding: "utf8",
      env: { ...process.env, PATH: `${failing}:${process.env.PATH}` },
    },
  );
  const output = `${result.stdout}${result.stderr}`;
  assert.notEqual(result.status, 0, output);
  assert.match(output, /simulated test-install failure/);
  assert.match(output, /did not install and run/);
  assert.match(output, /nothing was published/);
  assert.doesNotMatch(output, /would publish/);
  const commands = invocations();
  assert.ok(
    commands.some((line) => line.startsWith("npm install")),
    "the test-install was reached",
  );
  assert.ok(
    !commands.some((line) => /^(uv publish|npm publish|cargo publish|gh |curl )/.test(line)),
  );
});

test("the Linux job publishes exactly what it built, re-runs only what is missing, and installs it back", {
  skip,
}, () => {
  const planned = plan("linux");
  const file = (kind) => planned.find((artifact) => artifact.kind === kind).file;
  const env = registries();
  // A failed upload: the archive lands, its checksum is interrupted part of the way — GitHub
  // keeps a short asset under its name — and the step fails.
  control({
    releases: [tag],
    refuse: [{ registry: "github", file: file("checksum"), partial: true }],
  });
  expectStatus(
    recipe("linux", ["publish", "--to", "github", "--tag", tag, "--dist", dist], env),
    1,
    "a refused upload",
  );
  const first = requests().filter((r) => r.action === "upload");
  assert.deepEqual(
    first.map((r) => [r.file, r.refused]),
    [
      [file("archive"), false],
      [file("checksum"), true],
    ],
  );
  // Re-run: only what is missing is sent, the interrupted asset taken back first.
  control({ releases: [tag] });
  let mark = requests().length;
  expectStatus(
    recipe("linux", ["publish", "--to", "github", "--tag", tag, "--dist", dist], env),
    0,
    "the re-run",
  );
  assert.deepEqual(
    requests()
      .slice(mark)
      .filter((r) => ["upload", "delete-asset"].includes(r.action))
      .map((r) => [r.action, r.file]),
    [
      ["delete-asset", file("checksum")],
      ["upload", file("checksum")],
    ],
  );
  for (const registry of ["pypi", "npm"]) {
    mark = requests().length;
    expectStatus(
      recipe("linux", ["publish", "--to", registry, "--tag", tag, "--dist", dist], env),
      0,
      registry,
    );
    const sent = requests()
      .slice(mark)
      .filter((r) => ["upload", "publish"].includes(r.action));
    if (registry === "pypi")
      assert.deepEqual(
        sent.map((r) => [r.file, r.version]),
        [[file("wheel"), version]],
      );
    else
      assert.deepEqual(
        sent.map((r) => [r.package, r.versions]),
        [["@onetaskgraph/cli-linux-x64", [version]]],
      );
    mark = requests().length;
    expectStatus(
      recipe("linux", ["publish", "--to", registry, "--tag", tag, "--dist", dist], env),
      0,
      `${registry} again`,
    );
    assert.deepEqual(
      requests()
        .slice(mark)
        .filter((r) => ["upload", "publish"].includes(r.action)),
      [],
    );
  }
  // The bytes each registry holds are the bytes the build staged.
  const main = `http://127.0.0.1:${ports.main}`;
  const fetched = (url) => execFileSync("curl", ["-fsS", url], { maxBuffer: 1 << 28 });
  const hash = (data) => createHash("sha256").update(data).digest("hex");
  assert.equal(
    hash(fetched(`${main}/github/download/${tag}/${file("archive")}`)),
    sha256(join(dist, "assets", file("archive"))),
  );
  assert.equal(
    hash(fetched(`${main}/pypi/files/${file("wheel")}`)),
    sha256(join(dist, "wheels/x86_64-unknown-linux-gnu", file("wheel"))),
  );
  assert.equal(
    hash(fetched(`${main}/npm/@onetaskgraph/cli-linux-x64/-/cli-linux-x64-${version}.tgz`)),
    sha256(join(dist, "carriers/linux-x64", file("npm-platform"))),
  );
  for (const registry of ["github", "pypi", "npm"])
    expectStatus(
      recipe("linux", ["verify", "--from", registry, "--tag", tag, "--wait", "0"], env),
      0,
      `verify ${registry}`,
    );
});

test("the Linux job's registry verification fails on another version, an unusable package or none", {
  skip,
}, () => {
  const planned = plan("linux");
  const file = (kind) => planned.find((artifact) => artifact.kind === kind).file;
  const env = registries();
  const served = {
    github: { missing: file("archive"), unusable: file("archive") },
    pypi: { missing: file("wheel"), unusable: file("wheel") },
    npm: {
      missing: `@onetaskgraph/cli-linux-x64@${version}`,
      unusable: `cli-linux-x64-${version}.tgz`,
    },
  };
  for (const [registry, names] of Object.entries(served))
    for (const [why, state] of [
      ["serves another version", { releases: [tag], relabel: { from: version, to: "0.0.1" } }],
      ["serves no package for this platform", { releases: [tag], hide: [names.missing] }],
      ["serves an unusable package", { releases: [tag], corrupt: [names.unusable] }],
    ]) {
      control(state);
      const result = recipe(
        "linux",
        ["verify", "--from", registry, "--tag", tag, "--wait", "0"],
        env,
      );
      assert.equal(
        result.status,
        1,
        `${registry} ${why} and verification passed:\n${result.output.slice(-2000)}`,
      );
      assert.match(result.output, /release-cd: next: /, `${registry} ${why}`);
    }
  control({ releases: [tag] });
});

// The macOS and Windows jobs' publications, run for real over files bearing their planned
// names: what the publish job reads is that every platform is there.
function standIns(platform) {
  const directory = join(scratch, `dist-${platform}`);
  execFileSync(
    "python3",
    [
      "-c",
      `import hashlib, io, json, sys, tarfile, zipfile
from pathlib import Path
sys.path.insert(0, 'scripts')
from release_cd import plan, staged_path
platform, tag, root = sys.argv[1], sys.argv[2], Path(sys.argv[3])
version = tag[1:]
entries = []
artifacts = plan(platform, tag)
for artifact in artifacts:
    path = staged_path(root, artifact)
    path.parent.mkdir(parents=True, exist_ok=True)
    binary = 'onetaskgraph.exe' if platform == 'windows' else 'onetaskgraph'
    payload = f'stand-in {platform} binary'.encode()
    if artifact.kind == 'archive':
        if path.suffix == '.zip':
            with zipfile.ZipFile(path, 'w') as packed:
                packed.writestr(binary, payload)
        else:
            with tarfile.open(path, 'w:gz') as packed:
                info = tarfile.TarInfo(binary); info.size = len(payload); info.mode = 0o755
                packed.addfile(info, io.BytesIO(payload))
    elif artifact.kind == 'wheel':
        with zipfile.ZipFile(path, 'w') as packed:
            packed.writestr(f'onetaskgraph_cli-{version}.data/scripts/{binary}', payload)
            packed.writestr(f'onetaskgraph_cli-{version}.dist-info/METADATA', f'Metadata-Version: 2.1\\nName: onetaskgraph-cli\\nVersion: {version}\\n')
            packed.writestr(f'onetaskgraph_cli-{version}.dist-info/WHEEL', f'Wheel-Version: 1.0\\nRoot-Is-Purelib: false\\nTag: {artifact.wheel_tags[0]}\\n')
            packed.writestr(f'onetaskgraph_cli-{version}.dist-info/RECORD', '')
    elif artifact.kind == 'npm-platform':
        npm = path.parent.name
        manifest = json.loads(Path(f'npm/platforms/{npm}/package.json').read_text())
        with tarfile.open(path, 'w:gz') as packed:
            for name, data in [('package/package.json', json.dumps({**manifest, 'version': version}).encode()), (f'package/bin/{binary}', payload)]:
                info = tarfile.TarInfo(name); info.size = len(data); info.mode = 0o755
                packed.addfile(info, io.BytesIO(data))
for artifact in artifacts:
    if artifact.kind == 'checksum':
        archive = staged_path(root, next(a for a in artifacts if a.kind == 'archive'))
        staged_path(root, artifact).write_text(hashlib.sha256(archive.read_bytes()).hexdigest() + '  ' + archive.name + '\\n')
for artifact in artifacts:
    entries.append({**artifact.record(), 'sha256': hashlib.sha256(staged_path(root, artifact).read_bytes()).hexdigest()})
(root / f'release-{platform}.json').write_text(json.dumps({'tag': tag, 'platform': platform, 'artifacts': entries}))`,
      platform,
      tag,
      directory,
    ],
    { cwd: checkout },
  );
  return directory;
}

test("the publish job waits for every platform, then publishes the crates, SDKs and launcher once", {
  skip,
}, () => {
  const env = registries();
  control({ releases: [tag] });
  // Only Linux has published: every registry is missing a platform, and the check says which.
  for (const registry of ["github", "pypi", "npm"]) {
    const result = recipe(
      "publish",
      ["available", "--from", registry, "--tag", tag, "--wait", "0"],
      env,
    );
    assert.equal(
      result.status,
      1,
      `${registry} with two platforms missing:\n${result.output.slice(-2000)}`,
    );
    assert.match(result.output, /aarch64-apple-darwin|darwin-arm64/);
  }
  for (const platform of ["macos", "windows"]) {
    const directory = standIns(platform);
    for (const registry of ["github", "pypi", "npm"])
      expectStatus(
        recipe(platform, ["publish", "--to", registry, "--tag", tag, "--dist", directory], env),
        0,
        `${platform} to ${registry}`,
      );
  }
  for (const registry of ["github", "pypi", "npm"])
    expectStatus(
      recipe("publish", ["available", "--from", registry, "--tag", tag, "--wait", "0"], env),
      0,
      `${registry} with every platform`,
    );
  // A platform there but unusable is refused like a missing one: an archive that is not its
  // checksum's, a wheel or an npm package that is not what was published.
  const macos = Object.fromEntries(plan("macos").map((artifact) => [artifact.kind, artifact.file]));
  for (const [registry, corrupt] of [
    ["github", macos.archive],
    ["pypi", macos.wheel],
    ["npm", `cli-darwin-arm64-${version}.tgz`],
  ]) {
    control({ releases: [tag], corrupt: [corrupt] });
    const result = recipe(
      "publish",
      ["available", "--from", registry, "--tag", tag, "--wait", "0"],
      env,
    );
    assert.equal(
      result.status,
      1,
      `${registry} serving a corrupt macOS artifact:\n${result.output.slice(-2000)}`,
    );
    assert.match(result.output, /release-cd: next: re-run/);
  }
  // Readable but wrong: a wheel naming another version, and npm packages naming another
  // version or carrying no binary, each served under the macOS name.
  const wrong = join(scratch, "wrong");
  mkdirSync(wrong, { recursive: true });
  execFileSync("python3", [
    "-c",
    `import io, json, sys, tarfile, zipfile
from pathlib import Path
out, version = Path(sys.argv[1]), sys.argv[2]
with zipfile.ZipFile(out / 'wheel.whl', 'w') as packed:
    packed.writestr('onetaskgraph_cli-0.0.1.dist-info/METADATA', 'Metadata-Version: 2.1\\nName: onetaskgraph-cli\\nVersion: 0.0.1\\n')
for name, manifest, binary in [('version.tgz', {'name': '@onetaskgraph/cli-darwin-arm64', 'version': '0.0.1'}, True), ('binary.tgz', {'name': '@onetaskgraph/cli-darwin-arm64', 'version': version}, False)]:
    with tarfile.open(out / name, 'w:gz') as packed:
        for path, data in [('package/package.json', json.dumps(manifest).encode())] + ([('package/bin/onetaskgraph', b'stand-in')] if binary else []):
            info = tarfile.TarInfo(path); info.size = len(data)
            packed.addfile(info, io.BytesIO(data))`,
    wrong,
    version,
  ]);
  const carrierFile = `cli-darwin-arm64-${version}.tgz`;
  for (const [registry, replace, reason] of [
    [
      "pypi",
      { [macos.wheel]: join(wrong, "wheel.whl") },
      /as served is onetaskgraph-cli 0\.0\.1|has an invalid wheel|inconsistent version/,
    ],
    ["npm", { [carrierFile]: join(wrong, "version.tgz") }, /npm serves .*0\.0\.1/],
    ["npm", { [carrierFile]: join(wrong, "binary.tgz") }, /as served carries no binary/],
  ]) {
    control({ releases: [tag], replace });
    const result = recipe(
      "publish",
      ["available", "--from", registry, "--tag", tag, "--wait", "0"],
      env,
    );
    assert.equal(
      result.status,
      1,
      `${registry} ${JSON.stringify(replace)}:\n${result.output.slice(-2000)}`,
    );
    assert.match(result.output, reason);
  }
  control({ releases: [tag] });
  // Nothing platform-independent reached a registry before this point.
  const independentNames = plan("independent").map((artifact) => artifact.package);
  assert.ok(
    !requests().some(
      (r) => ["upload", "publish"].includes(r.action) && independentNames.includes(r.package),
    ),
  );
  const independent = join(scratch, "dist-independent");
  expectStatus(
    recipe("publish", ["build", "--tag", tag, "--dist", independent], env),
    0,
    "the publish job's build",
  );
  // What cargo and npm pack at publication is held to what the build checked: a crate's
  // sources, the launcher or the SDK changed since are refused before anything is sent.
  for (const [registry, path, packedFrom] of [
    ["crates", "README.md", /the source onetaskgraph-plugin-api-[\d.]+\.crate is packed from/],
    [
      "npm",
      "npm/cli/bin/onetaskgraph.js",
      /the source onetaskgraph-cli-[\d.]+\.tgz is packed from/,
    ],
    [
      "npm",
      "sdks/typescript/dist/index.js",
      /the source onetaskgraph-sdk-[\d.]+\.tgz is packed from/,
    ],
  ]) {
    const changed = join(checkout, path);
    const original = readFileSync(changed);
    writeFileSync(
      changed,
      Buffer.concat([original, Buffer.from("\n// changed after the build\n")]),
    );
    try {
      const before = requests().length;
      const moved = recipe(
        "publish",
        ["publish", "--to", registry, "--tag", tag, "--dist", independent],
        env,
      );
      assert.equal(moved.status, 1, `${path}: ${moved.output}`);
      assert.match(moved.output, packedFrom, path);
      assert.match(moved.output, /is not what the build produced/, path);
      assert.deepEqual(
        requests()
          .slice(before)
          .filter((r) => ["publish", "upload"].includes(r.action)),
        [],
        path,
      );
    } finally {
      writeFileSync(changed, original);
    }
  }
  // A crate refused part of the way through: those before it land, and the re-run sends only
  // the rest.
  control({ releases: [tag], refuse: [{ registry: "crates", package: "onetaskgraph-core" }] });
  let mark = requests().length;
  expectStatus(
    recipe("publish", ["publish", "--to", "crates", "--tag", tag, "--dist", independent], env),
    1,
    "a refused crate",
  );
  const order = plan("independent")
    .filter((a) => a.registry === "crates")
    .map((a) => a.package);
  const sent = () =>
    requests()
      .slice(mark)
      .filter((r) => r.action === "publish" && r.registry === "crates");
  assert.deepEqual(
    sent().map((r) => [r.package, r.version, r.refused]),
    [
      ...order.slice(0, 6).map((name) => [name, version, false]),
      ["onetaskgraph-core", version, true],
    ],
  );
  control({ releases: [tag] });
  mark = requests().length;
  expectStatus(
    recipe("publish", ["publish", "--to", "crates", "--tag", tag, "--dist", independent], env),
    0,
    "the crates re-run",
  );
  assert.deepEqual(
    sent().map((r) => r.package),
    order.slice(6),
  );
  // The SDK's sdist refused after its wheel landed (uv sends wheels first); the re-run sends
  // the sdist alone.
  const [sdist, sdkWheel] = plan("independent")
    .filter((a) => a.registry === "pypi")
    .map((a) => a.file);
  control({ releases: [tag], refuse: [{ registry: "pypi", file: sdist, status: 403 }] });
  mark = requests().length;
  expectStatus(
    recipe("publish", ["publish", "--to", "pypi", "--tag", tag, "--dist", independent], env),
    1,
    "a refused SDK sdist",
  );
  const uploads = () =>
    requests()
      .slice(mark)
      .filter((r) => r.action === "upload")
      .map((r) => [r.file, r.refused]);
  assert.deepEqual(uploads(), [
    [sdkWheel, false],
    [sdist, true],
  ]);
  control({ releases: [tag] });
  mark = requests().length;
  expectStatus(
    recipe("publish", ["publish", "--to", "pypi", "--tag", tag, "--dist", independent], env),
    0,
    "the Python SDK re-run",
  );
  assert.deepEqual(uploads(), [[sdist, false]]);
  for (const artifact of plan("independent").filter((a) => a.registry === "pypi"))
    assert.ok(existsSync(join(independent, "python-sdk", artifact.file)), artifact.file);
  // The TypeScript SDK refused after the launcher landed; the re-run sends the SDK alone.
  control({
    releases: [tag],
    refuse: [{ registry: "npm", package: "@onetaskgraph/sdk", status: 403 }],
  });
  mark = requests().length;
  expectStatus(
    recipe("publish", ["publish", "--to", "npm", "--tag", tag, "--dist", independent], env),
    1,
    "a refused TypeScript SDK",
  );
  const published = () =>
    requests()
      .slice(mark)
      .filter((r) => r.action === "publish")
      .map((r) => [r.package, r.versions, r.refused]);
  assert.deepEqual(published(), [
    ["@onetaskgraph/cli", [version], false],
    ["@onetaskgraph/sdk", [version], true],
  ]);
  control({ releases: [tag] });
  mark = requests().length;
  expectStatus(
    recipe("publish", ["publish", "--to", "npm", "--tag", tag, "--dist", independent], env),
    0,
    "the launcher and TypeScript SDK re-run",
  );
  assert.deepEqual(published(), [["@onetaskgraph/sdk", [version], false]]);
  for (const registry of ["crates", "pypi", "npm"])
    expectStatus(
      recipe("publish", ["verify", "--from", registry, "--tag", tag, "--wait", "0"], env),
      0,
      `the publish job's ${registry} check`,
    );
  // And it fails when a registry does not serve one of them, until it does again.
  const sdk = plan("independent")
    .filter((a) => a.registry === "pypi")
    .map((a) => a.file);
  for (const [registry, hide] of [
    ["crates", [`onetaskgraph@${version}`]],
    ["pypi", sdk],
    ["npm", [`@onetaskgraph/sdk@${version}`]],
  ]) {
    control({ releases: [tag], hide });
    const missing = recipe(
      "publish",
      ["verify", "--from", registry, "--tag", tag, "--wait", "0"],
      env,
    );
    assert.equal(
      missing.status,
      1,
      `${registry} missing a package:\n${missing.output.slice(-2000)}`,
    );
    assert.match(missing.output, /release-cd: next: /);
    control({ releases: [tag] });
    expectStatus(
      recipe("publish", ["verify", "--from", registry, "--tag", tag, "--wait", "0"], env),
      0,
      `the publish job's ${registry} check once it is served again`,
    );
  }
});

test("the GitHub upload refuses a missing token, a malformed release and an untrusted upload host", {
  skip,
}, () => {
  for (const [name, env, state, reason] of [
    ["no token", { GH_TOKEN: "", GITHUB_TOKEN: "" }, { releases: [tag] }, /no GitHub token/],
    [
      "a malformed release",
      {},
      { releases: [tag], release_answer: { assets: "none" } },
      /is not a release/,
    ],
    [
      "a malformed asset",
      {},
      {
        releases: [tag],
        release_answer: {
          tag_name: tag,
          upload_url: "http://127.0.0.1:9/u",
          assets: [{ name: 5 }],
        },
      },
      /not a release asset/,
    ],
    [
      "another tag's release",
      {},
      {
        releases: [tag],
        release_answer: { tag_name: "v0.0.1", upload_url: "http://127.0.0.1:9/u", assets: [] },
      },
      /it is the release of 'v0\.0\.1'/,
    ],
    [
      "an untrusted upload host",
      {},
      { releases: [tag], upload_host: "http://198.51.100.7:9/uploads" },
      /neither GitHub's API nor its upload host/,
    ],
  ]) {
    control(state);
    const mark = requests().length;
    const result = recipe(
      "macos",
      ["publish", "--to", "github", "--tag", tag, "--dist", join(scratch, "dist-macos")],
      {
        ...registries(),
        ...env,
      },
    );
    assert.equal(result.status, 1, `${name}:\n${result.output}`);
    assert.match(result.output, reason, name);
    assert.deepEqual(
      requests()
        .slice(mark)
        .filter((r) => r.action === "upload"),
      [],
      name,
    );
  }
  control({ releases: [tag] });
});

test("a publication refuses a build it cannot vouch for before it sends anything", { skip }, () => {
  const wheel = plan("linux").find((artifact) => artifact.kind === "wheel");
  const manifest = JSON.parse(readFileSync(join(dist, "release-linux.json"), "utf8"));
  const copy = (name, change) => {
    const directory = join(scratch, `refused-${name.replaceAll(" ", "-")}`);
    execFileSync("cp", ["-R", dist, directory]);
    change(directory);
    return directory;
  };
  const rewrite = (directory, value) =>
    writeFileSync(join(directory, "release-linux.json"), JSON.stringify(value));
  const cases = [
    ["no build", (d) => rmSync(join(d, "release-linux.json")), /no usable build of linux/],
    [
      "another tag",
      (d) => rewrite(d, { ...manifest, tag: "v0.0.1" }),
      /records a linux build of v0\.0\.1/,
    ],
    [
      "a file the build did not produce",
      (d) => writeFileSync(join(d, "wheels/x86_64-unknown-linux-gnu", wheel.file), "tampered"),
      /is not what the build produced/,
    ],
    [
      "a file with no recorded digest",
      (d) =>
        rewrite(d, {
          ...manifest,
          artifacts: manifest.artifacts.map(({ sha256: _, ...entry }) => entry),
        }),
      /is not what the build produced/,
    ],
    [
      "a list that is not the plan",
      (d) => rewrite(d, { ...manifest, artifacts: manifest.artifacts.slice(1) }),
      /does not list exactly the files linux releases/,
    ],
  ];
  for (const [name, change, reason] of cases) {
    writeFileSync(log, "");
    const result = recipe(
      "linux",
      ["publish", "--to", "pypi", "--tag", tag, "--dist", copy(name, change)],
      registries(),
    );
    assert.equal(result.status, 1, `${name}:\n${result.output}`);
    assert.match(result.output, reason, name);
    assert.match(result.output, /next: run `just release-linux build/, name);
    assert.ok(!invocations().some((line) => line.startsWith("uv publish")), `${name} reached uv`);
  }
});

test("registry verification waits out a registry that has not caught up, and fails at its deadline", {
  skip,
}, async () => {
  const wheel = plan("linux").find((artifact) => artifact.kind === "wheel").file;
  control({ releases: [tag], hide: [wheel] });
  const deadline = recipe(
    "linux",
    ["verify", "--from", "pypi", "--tag", tag, "--wait", "1"],
    registries(),
  );
  // Past its deadline the step fails, naming what the index lacks, rather than waiting on.
  assert.equal(deadline.status, 1, deadline.output);
  assert.match(deadline.output, /No matching distribution found for onetaskgraph-cli==/);
  assert.match(deadline.output, /next: check what the index serves/);
  const waiting = spawn(
    "just",
    ["release-linux", "verify", "--from", "pypi", "--tag", tag, "--wait", "120"],
    { cwd: checkout, env: { ...process.env, ...registries() } },
  );
  let output = "";
  for (const stream of [waiting.stdout, waiting.stderr])
    stream.on("data", (chunk) => {
      output += chunk;
    });
  const exited = new Promise((resolve) => waiting.on("exit", resolve));
  while (!output.includes("asking again")) await new Promise((resolve) => setTimeout(resolve, 200));
  control({ releases: [tag] });
  assert.equal(await exited, 0, output);
  assert.match(output, /installed v[\d.]+ for x86_64-unknown-linux-gnu from pypi and ran it/);
});

test("the npm publication refuses a package set or platform it does not publish", {
  skip:
    process.platform === "win32" && "a Linux-shaped scratch directory stands in for the carriers",
}, () => {
  for (const [env, reason] of [
    [{ NPM_PUBLISH_SET: "everything" }, /NPM_PUBLISH_SET must be all, carriers or root/],
    [
      { NPM_PUBLISH_SET: "carriers", NPM_PLATFORMS: "../platforms/linux-x64" },
      /not one of the platforms under npm\/platforms/,
    ],
    [
      { NPM_PUBLISH_SET: "carriers", NPM_PLATFORMS: "linux-arm64" },
      /not one of the platforms under npm\/platforms/,
    ],
  ]) {
    const result = spawnSync("bash", ["scripts/publish-npm.sh"], {
      cwd: checkout,
      encoding: "utf8",
      env: {
        ...process.env,
        NODE_AUTH_TOKEN: "fixture-token",
        NPM_CARRIERS: scratch,
        NPM_REGISTRY: "http://127.0.0.1:9/",
        ...env,
      },
    });
    assert.equal(result.status, 64, result.stderr);
    assert.match(result.stderr, reason);
    assert.match(result.stderr, /next: /);
  }
});
