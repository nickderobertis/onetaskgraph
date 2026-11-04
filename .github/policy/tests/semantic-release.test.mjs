import "../../test-setup.mjs";
import assert from "node:assert/strict";
import { existsSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { test } from "node:test";
import { base, files, heads, history, revision, scratchRoot, semantic } from "./policy-fixture.mjs";

test("the real semantic CLI accepts release edits and rejects source and dependency changes", () => {
  assert.equal(semantic(heads.version).isReleaseOnly, true);
  for (const head of [heads.source, heads.dependency])
    assert.equal(semantic(head).isReleaseOnly, false);
});

test("semantic comparison keeps version-only agreement when the host spells paths with backslashes", () => {
  // The Windows runner's pathlib renders the registered version files as
  // `sdks\python\pyproject.toml`; git names them with forward slashes. This
  // python keeps the real files and real comparison, and spells only str() the
  // Windows way, as that runner does.
  const shim = mkdtempSync(resolve(scratchRoot, "windows-paths-"));
  try {
    writeFileSync(
      resolve(shim, "sitecustomize.py"),
      [
        "import sys",
        `sys.path.insert(0, ${JSON.stringify(resolve(history, "scripts"))})`,
        "import product_versions",
        "class WindowsSpelledPath:",
        "    def __init__(self, real): self._real = real",
        "    def __str__(self): return str(self._real).replace('/', '\\\\')",
        "    def __fspath__(self): return str(self._real)",
        "    def __getattr__(self, name): return getattr(self._real, name)",
        "for entry in product_versions.RECONCILED_VERSION_FILES:",
        "    object.__setattr__(entry, 'path', WindowsSpelledPath(entry.path))",
        // Python reports a failing sitecustomize and carries on, so the shim
        // proves it took by leaving a marker only once paths spell the Windows way.
        "if '\\\\' in str(product_versions.RECONCILED_VERSION_FILES[-1].path):",
        `    open(${JSON.stringify(resolve(shim, "took"))}, 'w').close()`,
        "",
      ].join("\n"),
    );
    const decision = semantic(heads.version, base, {
      PYTHONPATH: shim,
    });
    assert.deepEqual(decision.isReleaseOnly, true);
    assert.ok(existsSync(resolve(shim, "took")), "Windows path simulation did not take");
    assert.equal(semantic(heads.source, base, { PYTHONPATH: shim }).isReleaseOnly, false);
  } finally {
    rmSync(shim, { recursive: true, force: true });
  }
});

test("semantic comparison refuses mixed fields, dependency lock versions and unrelated changelogs", () => {
  const changes = [
    {
      "Cargo.lock": files["Cargo.lock"].replace(
        'name = "external"\nversion = "1.2.3"',
        'name = "external"\nversion = "1.2.4"',
      ),
    },
    { "Cargo.lock": files["Cargo.lock"].replace('checksum = "abc"', 'checksum = "def"') },
    { "bun.lock": files["bun.lock"].replace("external@1.0.0", "external@2.0.0") },
    {
      "sdks/typescript/src/index.ts": files["sdks/typescript/src/index.ts"].replace(
        "OTHER = 1",
        "OTHER = 2",
      ),
    },
    {
      "npm/cli/package.json": files["npm/cli/package.json"].replace(
        '@onetaskgraph/cli"',
        '@forged/cli"',
      ),
    },
    { "fixtures/CHANGELOG.md": "# unrelated" },
    { "crates/onetaskgraph-core/fixtures/CHANGELOG.md": "# fixture changes" },
    { "fixtures/Cargo.toml": '[package]\nversion = "1.2.4"' },
    { "pyproject.toml": "malformed" },
  ];
  for (const changed of changes)
    assert.equal(semantic(revision(changed)).isReleaseOnly, false, JSON.stringify(changed));
});

test("same-named registry packages retain their versions in semantic lock comparison", () => {
  for (const path of ["Cargo.lock", "uv.lock"]) {
    const source =
      path === "Cargo.lock"
        ? 'source = "registry+example"'
        : 'source = { registry = "https://pypi.org/simple" }';
    const record = `\n[[package]]\nname = "onetaskgraph-core"\nversion = "9.8.7"\n${source}\n`;
    const before = revision({ [path]: files[path] + record });
    const after = revision({ [path]: files[path] + record.replace("9.8.7", "9.8.8") });
    const decision = semantic(after, before);
    assert.equal(decision.isReleaseOnly, false, path);
  }
});

for (const field of ["dependency reference", "requires-dist specifier"]) {
  test(`registry ${field} cannot borrow a local product name`, () => {
    const dependency =
      field === "dependency reference"
        ? 'dependencies = [{ name = "onetaskgraph-core", version = "9.8.7", source = { registry = "https://pypi.org/simple" } }]'
        : '[package.metadata]\nrequires-dist = [{ name = "onetaskgraph-core", specifier = "==9.8.7" }]';
    const lock = `version = 1\n[[package]]\nname = "onetaskgraph-sdk"\nversion = "1.2.3"\nsource = { editable = "sdks/python" }\n${dependency}\n[[package]]\nname = "onetaskgraph-core"\nversion = "9.8.7"\nsource = { registry = "https://pypi.org/simple" }\n`;
    const before = revision({ "uv.lock": lock });
    const after = revision({
      "uv.lock": lock
        .replace('version = "9.8.7", source', 'version = "9.8.8", source')
        .replace('specifier = "==9.8.7"', 'specifier = "==9.8.8"'),
    });
    assert.equal(semantic(after, before).isReleaseOnly, false);
  });
}

test("lock references resolve explicit local sources and refuse ambiguous or absent identities", () => {
  for (const metadata of [false, true]) {
    for (const identity of ["explicit", "ambiguous", "absent"]) {
      const selector =
        identity === "explicit"
          ? metadata
            ? ', editable = "."'
            : ', source = { editable = "." }'
          : "";
      const reference = metadata
        ? `[package.metadata]\nrequires-dist = [{ name = "onetaskgraph-cli", specifier = "==1.2.3"${selector} }]`
        : `dependencies = [{ name = "onetaskgraph-cli", version = "1.2.3"${selector} }]`;
      const local =
        identity === "absent"
          ? ""
          : '\n[[package]]\nname = "onetaskgraph-cli"\nversion = "1.2.3"\nsource = { editable = "." }\n';
      const registry =
        identity === "absent"
          ? ""
          : '\n[[package]]\nname = "onetaskgraph-cli"\nversion = "1.2.3"\nsource = { registry = "https://pypi.org/simple" }\n';
      const beforeLock = `version = 1\n[[package]]\nname = "onetaskgraph-sdk"\nversion = "1.2.3"\nsource = { editable = "sdks/python" }\n${reference}\n${local}${registry}`;
      const afterLock = beforeLock
        .replace(reference, reference.replaceAll("1.2.3", "1.2.4"))
        .replace(local || "never-match", local.replace("1.2.3", "1.2.4"));
      const before = revision({ "uv.lock": beforeLock });
      const after = revision({ "uv.lock": afterLock });
      const result = semantic(after, before);
      assert.equal(result.isReleaseOnly, identity === "explicit", `${metadata}/${identity}`);
    }
  }
});
