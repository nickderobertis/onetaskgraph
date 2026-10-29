import assert from "node:assert/strict";
import { execFileSync, execFile } from "node:child_process";
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
  rmSync,
  symlinkSync,
  readdirSync,
} from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { promisify } from "node:util";
import { parse } from "yaml";
import { read } from "../workflow-graph.mjs";
import { test } from "node:test";
import { workflows } from "../workflow-graph.mjs";

function cleanupProblems(all) {
  const problems = [];
  for (const [file, workflow] of Object.entries(all)) {
    for (const [id, job] of Object.entries(workflow.jobs ?? {})) {
      const steps = job.steps ?? [];
      for (const [index, step] of steps.entries()) {
        if (
          !step.uses?.startsWith("actions-rust-lang/setup-rust-toolchain@") ||
          step.with?.cache === false ||
          step.with?.cache === "false"
        )
          continue;
        const cleanup = steps[index - 1];
        if (
          cleanup?.run !== "rustup toolchain uninstall stable" ||
          cleanup.shell !== "bash" ||
          cleanup.if !== step.if ||
          cleanup["continue-on-error"]
        )
          problems.push(`${file}:${id} must remove floating stable before Rust cache restore`);
      }
    }
  }
  return problems;
}

const all = () => {
  const definitions = workflows();
  return Object.fromEntries(
    Object.entries(definitions).filter(
      ([file]) => file === "ci.yml" || file === "rust-cache-warm.yml",
    ),
  );
};

test("every Rust cache consumer and main-push producer excludes floating stable", () => {
  assert.deepEqual(cleanupProblems(all()), []);
});

test("removing, moving or disabling cleanup in any Rust cache job is refused", () => {
  const original = all();
  let checked = 0;
  for (const [file, workflow] of Object.entries(original)) {
    for (const [id, job] of Object.entries(workflow.jobs ?? {})) {
      const index = job.steps?.findIndex(
        (step) => step.run === "rustup toolchain uninstall stable",
      );
      if (index === undefined || index < 0) continue;
      for (const mutation of ["remove", "move", "disable", "ignore-failure"]) {
        const changed = structuredClone(original);
        const steps = changed[file].jobs[id].steps;
        if (mutation === "remove") steps.splice(index, 1);
        if (mutation === "move") steps.push(...steps.splice(index, 1));
        if (mutation === "disable") steps[index].if = "false";
        if (mutation === "ignore-failure") steps[index]["continue-on-error"] = true;
        assert.ok(cleanupProblems(changed).some((problem) => problem.startsWith(`${file}:${id} `)));
      }
      checked++;
    }
  }
  assert.ok(checked > 0, "must exercise real cache jobs");
});

// Validate the upstream declarations this journey interprets; a changed action
// contract must fail explicitly rather than silently exercising different inputs.
function actionManifest(text) {
  const manifest = parse(text);
  const record = (value) => value !== null && typeof value === "object" && !Array.isArray(value);
  assert.ok(
    record(manifest) && record(manifest.inputs) && record(manifest.runs),
    "invalid action manifest",
  );
  for (const [name, input] of Object.entries(manifest.inputs)) {
    assert.match(name, /^[a-z][a-z0-9-]*$/);
    assert.ok(record(input), `invalid input declaration: ${name}`);
    assert.ok(
      input.default === undefined ||
        typeof input.default === "string" ||
        typeof input.default === "boolean",
      `invalid input default: ${name}`,
    );
  }
  return manifest;
}

// Run the distributed action unchanged against a loopback cache miss. Rustup and
// both compilers are real; the remote cache service alone is replaced by a loopback peer.
test("the real cache action ignores floating stable but tracks the pin and lockfile", async () => {
  const scratch = mkdtempSync(join(tmpdir(), "rust-cache-key-"));
  const run = promisify(execFile);
  const command = (program, args, options = {}) =>
    execFileSync(program, args, { encoding: "utf8", ...options }).trim();
  const server = createServer((_request, response) => {
    response.writeHead(204);
    response.end();
  });
  try {
    await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
    const setup = actionManifest(
      command("curl", [
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "https://raw.githubusercontent.com/actions-rust-lang/setup-rust-toolchain/v1/action.yml",
      ]),
    );
    assert.ok(Array.isArray(setup.runs.steps), "setup action must declare steps");
    for (const step of setup.runs.steps) {
      assert.ok(
        step !== null && typeof step === "object" && !Array.isArray(step),
        "invalid setup step",
      );
      for (const field of ["uses", "name", "run"])
        assert.ok(
          step[field] === undefined || typeof step[field] === "string",
          `invalid step ${field}`,
        );
    }
    const cacheStep = setup.runs.steps.find((step) =>
      step.uses?.startsWith("Swatinem/rust-cache@"),
    );
    assert.ok(cacheStep, "setup action must declare its cache action");
    assert.match(cacheStep.uses, /^Swatinem\/rust-cache@[0-9a-f]{40}$/);
    assert.ok(
      cacheStep.with !== null &&
        typeof cacheStep.with === "object" &&
        !Array.isArray(cacheStep.with),
      "invalid forwarded cache inputs",
    );
    const revision = cacheStep.uses.split("@")[1];
    const action = join(scratch, "action");
    command("git", ["init", "-q", action]);
    command("git", [
      "-C",
      action,
      "fetch",
      "--depth=1",
      "https://github.com/Swatinem/rust-cache",
      revision,
    ]);
    command("git", ["-C", action, "checkout", "-q", "FETCH_HEAD"]);
    const manifest = actionManifest(readFileSync(join(action, "action.yml"), "utf8"));
    assert.equal(manifest.runs.using, "node24");
    assert.equal(typeof manifest.runs.main, "string");
    assert.match(manifest.runs.main, /^dist\/(?:[A-Za-z0-9_-]+\/)*[A-Za-z0-9_-]+\.js$/);
    const pinMatch = read("rust-toolchain.toml").match(/^channel = "([0-9]+\.[0-9]+\.[0-9]+)"$/m);
    assert.ok(pinMatch, "journey requires an explicit pinned Rust release");
    const pin = pinMatch[1];
    const other = "1.99.0";
    assert.notEqual(pin, other);
    const rustupHome = join(scratch, "rustup");
    const toolchains = join(rustupHome, "toolchains");
    mkdirSync(toolchains, { recursive: true });
    const hostMatch = command("rustc", ["-vV"]).match(/^host: ([A-Za-z0-9_-]+)$/m);
    assert.ok(hostMatch, "rustc must report a valid host triple");
    const host = hostMatch[1];
    for (const version of [pin, other]) {
      // Reuse an installed compiler where available; otherwise install into this
      // test's disposable rustup home, never into the contributor's toolchains.
      let compiler;
      try {
        compiler = command("rustup", ["which", "--toolchain", version, "rustc"]);
      } catch {
        command(
          "rustup",
          ["toolchain", "install", version, "--profile", "minimal", "--no-self-update"],
          { env: { ...process.env, RUSTUP_HOME: rustupHome } },
        );
        continue;
      }
      symlinkSync(
        dirname(dirname(compiler)),
        join(toolchains, `${version}-${host}`),
        process.platform === "win32" ? "junction" : "dir",
      );
    }
    const workspace = join(scratch, "workspace");
    mkdirSync(join(workspace, "src"), { recursive: true });
    writeFileSync(
      join(workspace, "Cargo.toml"),
      '[package]\nname = "cache-journey"\nversion = "0.1.0"\nedition = "2021"\n',
    );
    writeFileSync(join(workspace, "src/lib.rs"), "pub fn example() {}\n");
    const env = Object.fromEntries(
      Object.entries(process.env).filter(
        ([key]) => !/^(CARGO|RUST|INPUT_|ACTIONS_|GITHUB_)/.test(key),
      ),
    );
    Object.assign(env, {
      RUSTUP_HOME: rustupHome,
      CARGO_HOME: join(scratch, "cargo"),
      GITHUB_WORKSPACE: workspace,
      GITHUB_JOB: "check",
      GITHUB_REPOSITORY: "example/cache-journey",
      GITHUB_REF: "refs/heads/main",
      GITHUB_SHA: "0".repeat(40),
      ACTIONS_CACHE_URL: `http://127.0.0.1:${server.address().port}/`,
      ACTIONS_RUNTIME_TOKEN: "loopback-only",
      RUNNER_OS:
        process.platform === "win32"
          ? "Windows"
          : process.platform === "darwin"
            ? "macOS"
            : "Linux",
      RUNNER_ARCH: process.arch === "arm64" ? "ARM64" : "X64",
      GITHUB_ENV: join(scratch, "env"),
      GITHUB_OUTPUT: join(scratch, "output"),
      GITHUB_STATE: join(scratch, "state"),
    });
    for (const name of ["GITHUB_ENV", "GITHUB_OUTPUT", "GITHUB_STATE"])
      writeFileSync(env[name], "");
    for (const [name, input] of Object.entries(manifest.inputs))
      if (input.default !== undefined) env[`INPUT_${name.toUpperCase()}`] = String(input.default);
    // Resolve the composite action's input forwarding using the same defaults
    // these jobs use (none supplies a toolchain/cache override).
    for (const [name, expression] of Object.entries(cacheStep.with)) {
      assert.ok(Object.hasOwn(manifest.inputs, name), `unknown cache input: ${name}`);
      assert.equal(typeof expression, "string");
      assert.match(
        expression,
        /^\$\{\{\s*inputs\.[a-z-]+(?:\s*\|\|\s*inputs\.[a-z-]+)*\s*\}\}$/,
        "unsupported forwarding expression",
      );
      const references = [...expression.matchAll(/inputs\.([a-z-]+)/g)];
      for (const [, input] of references)
        assert.ok(Object.hasOwn(setup.inputs, input), `unknown setup input: ${input}`);
      assert.ok(references.length, `unsupported cache input: ${expression}`);
      const value =
        references.map(([, input]) => setup.inputs[input]?.default ?? "").find(Boolean) ?? "";
      env[`INPUT_${name.toUpperCase()}`] = String(value);
    }
    // Apply the same environment-setting step as setup-rust-toolchain, with
    // its default rustflags input, before invoking its cache action.
    const environmentStep = setup.runs.steps.find(
      (step) => step.name === "Setting Environment Variables",
    );
    assert.ok(
      environmentStep && typeof environmentStep.run === "string",
      "setup action must declare its environment script",
    );
    assert.equal(typeof setup.inputs.rustflags?.default, "string");
    command("bash", ["-e", "-c", environmentStep.run], {
      cwd: workspace,
      env: { ...env, _srt_NEW_RUSTFLAGS: setup.inputs.rustflags.default },
    });
    for (const line of readFileSync(env.GITHUB_ENV, "utf8").trim().split(/\r?\n/)) {
      assert.match(
        line,
        /^[A-Z_][A-Z0-9_]*=[^\r\n]*$/,
        "unsupported setup action environment record",
      );
      const at = line.indexOf("=");
      env[line.slice(0, at)] = line.slice(at + 1);
    }
    const cleanup = all()["ci.yml"].jobs.check.steps.find(
      (step) => step.run === "rustup toolchain uninstall stable",
    );
    async function key(floating, pinned, clean) {
      for (const entry of readdirSync(toolchains))
        if (entry.startsWith("stable-"))
          rmSync(join(toolchains, entry), { recursive: true, force: true });
      symlinkSync(
        join(toolchains, `${floating}-${host}`),
        join(toolchains, `stable-${host}`),
        process.platform === "win32" ? "junction" : "dir",
      );
      writeFileSync(join(workspace, "rust-toolchain.toml"), `[toolchain]\nchannel = "${pinned}"\n`);
      if (clean) command("bash", ["-e", "-c", cleanup.run], { cwd: workspace, env });
      // The setup action resolves the toolchain file before restoring the cache.
      command("rustup", ["show", "active-toolchain"], { cwd: workspace, env });
      command("cargo", ["generate-lockfile", "--offline"], { cwd: workspace, env });
      const { stdout, stderr } = await run(process.execPath, [join(action, manifest.runs.main)], {
        cwd: workspace,
        env,
        maxBuffer: 4 * 1024 * 1024,
      });
      assert.doesNotMatch(stdout + stderr, /::error::/);
      const match = stdout.match(/Cache Key:\r?\n\s+(\S+)/);
      assert.ok(match, stdout + stderr);
      return match[1];
    }
    assert.notEqual(
      await key(pin, pin, false),
      await key(other, pin, false),
      "control: the runner image used to invalidate the key",
    );
    const corrected = await key(pin, pin, true);
    assert.equal(corrected, await key(other, pin, true));
    assert.notEqual(corrected, await key(other, other, true));
    await key(other, pin, true);
    writeFileSync(
      join(workspace, "Cargo.lock"),
      readFileSync(join(workspace, "Cargo.lock"), "utf8") +
        '\n[[package]]\nname = "lockfile-change"\nversion = "1.0.0"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\nchecksum = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"\n',
    );
    // Keep the modified lockfile intact: invoke the cache action directly.
    const { stdout, stderr } = await run(process.execPath, [join(action, manifest.runs.main)], {
      cwd: workspace,
      env,
      maxBuffer: 4 * 1024 * 1024,
    });
    assert.doesNotMatch(stdout + stderr, /::error::/);
    assert.ok(stdout.match(/Cache Key:\r?\n\s+(\S+)/), stdout + stderr);
    assert.notEqual(corrected, stdout.match(/Cache Key:\r?\n\s+(\S+)/)?.[1]);
  } finally {
    await new Promise((resolve) => server.close(resolve));
    rmSync(scratch, { recursive: true, force: true });
  }
});
