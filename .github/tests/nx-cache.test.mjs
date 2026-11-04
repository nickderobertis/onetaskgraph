import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, test } from "node:test";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../", import.meta.url));
const scratch = mkdtempSync(
  join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir(), "nx-cache-"),
);
after(() => rmSync(scratch, { recursive: true, force: true }));
let cases = 0;
const tree = () => {
  const dir = join(scratch, String(cases++));
  mkdirSync(dir);
  return dir;
};
function command(program, args, cwd, env = {}) {
  const result = spawnSync(program, args, {
    cwd,
    env: { ...process.env, ...env },
    encoding: "utf8",
    timeout: 30_000,
  });
  assert.equal(result.status, 0, result.stdout + result.stderr);
  return result.stdout.trim();
}

// Biome excludes exactly these upstream-owned copies; this drift test holds them byte-identical.
// Full-file digests from dero-skills PR #150's merge commit
// 63e071b72420bd6737226a50cb8a7a7760cc73a4, rather than the header's body digest.
const copies = {
  "scripts/nx-remote-cache.mjs": "4445c0d8d5f6045bb2b14b19cd03f9600b6b98f88dc689421d98d3ef996664b6",
  "scripts/nx-version-free-inputs.mjs":
    "fdd4f59fc98a0073dbefdbaa0f08f4b58432bbf8e55d315865f951e0151b19b2",
};
function drift(file, bytes) {
  return createHash("sha256").update(bytes).digest("hex") !== copies[file];
}
test("shared scripts remain byte-identical to the agreed upstream commit", () => {
  for (const file of Object.keys(copies)) {
    const bytes = readFileSync(join(root, file));
    assert.equal(drift(file, bytes), false, file);
    assert.equal(drift(file, Buffer.concat([bytes, Buffer.from("\n")])), true, file);
  }
  const config = JSON.parse(readFileSync(join(root, "biome.json"), "utf8"));
  for (const file of Object.keys(copies)) assert.ok(config.files.includes.includes(`!${file}`));
});

test("trust refuses writes for fork and Dependabot PRs, and permits a repository contributor", () => {
  const dir = tree();
  for (const [head, author, expected] of [
    ["contributor/fork", "contributor", "true"],
    ["nickderobertis/onetaskgraph", "dependabot[bot]", "true"],
    ["nickderobertis/onetaskgraph", "contributor", "false"],
  ]) {
    const event = join(dir, "event.json");
    const output = join(dir, "output");
    writeFileSync(
      event,
      JSON.stringify({
        pull_request: {
          base: { repo: { full_name: "nickderobertis/onetaskgraph" } },
          head: { repo: { full_name: head } },
          user: { login: author },
        },
      }),
    );
    writeFileSync(output, "");
    command(process.execPath, [join(root, "scripts/nx-remote-cache.mjs"), "trust"], dir, {
      GITHUB_EVENT_PATH: event,
      GITHUB_OUTPUT: output,
      GITHUB_REPOSITORY: "nickderobertis/onetaskgraph",
    });
    assert.equal(
      readFileSync(output, "utf8"),
      `read_only=${expected}\npromotable=${expected === "false"}\n`,
    );
  }
});

test("the version digest masks only the package version and keeps dependencies and feature inputs", () => {
  const dir = tree();
  mkdirSync(join(dir, "scripts"));
  copyFileSync(
    join(root, "scripts/nx-version-free-inputs.mjs"),
    join(dir, "scripts/nx-version-free-inputs.mjs"),
  );
  command("git", ["init", "--quiet"], dir);
  const manifest = (version, dependency = "1.0", feature = "[]") =>
    `[package]\nname = "fixture"\nversion = "${version}"\n[dependencies]\nserde = "${dependency}"\n[features]\ndefault = ${feature}\nextra = []\n`;
  const path = join(dir, "Cargo.toml");
  writeFileSync(path, manifest("1.0.0"));
  command("git", ["add", "."], dir);
  const digest = (keep = false) =>
    command(
      process.execPath,
      [
        "scripts/nx-version-free-inputs.mjs",
        "--ecosystem",
        "cargo",
        ...(keep ? ["--keep-version"] : []),
      ],
      dir,
    );
  const original = digest();
  const kept = digest(true);
  writeFileSync(path, manifest("1.0.1"));
  assert.equal(digest(), original);
  assert.notEqual(digest(true), kept);
  writeFileSync(path, manifest("1.0.1", "1.1"));
  assert.notEqual(digest(), original);
  writeFileSync(path, manifest("1.0.1", "1.0", '["extra"]'));
  assert.notEqual(digest(), original);
});
