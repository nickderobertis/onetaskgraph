import { expect, test } from "bun:test";
import { appendFileSync, chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";

const packageRoot = resolve(import.meta.dir, "..");
const binary = resolve(packageRoot, "../../target/debug/onetaskgraph");

function emitter(directory: string, name: string, output: string) {
  const windows = process.platform === "win32";
  const path = resolve(directory, `${name}${windows ? ".cmd" : ""}`);
  const quoted = JSON.stringify(output);
  const body = windows
    ? `@echo off\r\nnode -e "process.stdout.write(${JSON.stringify(quoted)})"\r\n`
    : `#!/usr/bin/env node\nprocess.stdout.write(${quoted});\n`;
  writeFileSync(path, body);
  if (!windows) chmodSync(path, 0o755);
  return path;
}

function generateWith(binaryPath: string, generated: string) {
  return spawnSync("bun", ["scripts/generate.ts"], {
    cwd: packageRoot,
    encoding: "utf8",
    env: {
      ...process.env,
      NODE_ENV: "test",
      ONETASKGRAPH_BIN: binaryPath,
      ONETASKGRAPH_GENERATED_DIR: generated,
    },
  });
}

test("generation, clean check, stale check, and invalid arguments use the real binary", () => {
  const generated = mkdtempSync(resolve(tmpdir(), "onetaskgraph-generated-"));
  const run = (...args: string[]) =>
    spawnSync("bun", ["scripts/generate.ts", ...args], {
      cwd: packageRoot,
      encoding: "utf8",
      env: {
        ...process.env,
        NODE_ENV: "test",
        ONETASKGRAPH_BIN: binary,
        ONETASKGRAPH_GENERATED_DIR: generated,
      },
    });
  try {
    const generatedResult = run();
    expect(generatedResult.status, generatedResult.stderr).toBe(0);
    expect(run("--check").status).toBe(0);
    appendFileSync(resolve(generated, "commands.ts"), "// stale\n");
    const stale = run("--check");
    expect(stale.status).toBe(1);
    expect(stale.stderr).toContain("commands.ts would change");
    expect(run().status).toBe(0);
    rmSync(resolve(generated, "commands.ts"));
    const missing = run("--check");
    expect(missing.status).toBe(1);
    expect(missing.stderr).toContain("commands.ts would change");
    expect(run("--unknown").status).toBe(1);
    const unavailable = spawnSync("bun", ["scripts/generate.ts"], {
      cwd: packageRoot,
      encoding: "utf8",
      env: { ...process.env, ONETASKGRAPH_BIN: resolve(generated, "missing-binary") },
    });
    expect(unavailable.status).toBe(1);
    expect(unavailable.stderr).toContain("could not emit the SDK contract");
  } finally {
    rmSync(generated, { recursive: true, force: true });
  }
});

test("generator rejects unsafe destinations and malformed executable output", () => {
  const fixtures = mkdtempSync(resolve(tmpdir(), "onetaskgraph-generator-boundary-"));
  try {
    const unsafe = spawnSync("bun", ["scripts/generate.ts"], {
      cwd: packageRoot,
      encoding: "utf8",
      env: {
        ...process.env,
        NODE_ENV: "test",
        ONETASKGRAPH_BIN: binary,
        ONETASKGRAPH_GENERATED_DIR: packageRoot,
      },
    });
    expect(unsafe.status).toBe(1);
    expect(unsafe.stderr).toContain("only accepted under the test temporary directory");

    const missingDestination = spawnSync("bun", ["scripts/generate.ts"], {
      cwd: packageRoot,
      encoding: "utf8",
      env: {
        ...process.env,
        NODE_ENV: "test",
        ONETASKGRAPH_BIN: binary,
        ONETASKGRAPH_GENERATED_DIR: resolve(fixtures, "missing"),
      },
    });
    expect(missingDestination.status).toBe(1);
    expect(missingDestination.stderr).toContain("could not resolve ONETASKGRAPH_GENERATED_DIR");
    expect(missingDestination.stderr).toContain("next: create the directory");

    const nonJson = generateWith(emitter(fixtures, "non-json", "not JSON"), fixtures);
    expect(nonJson.status).toBe(1);
    expect(nonJson.stderr).toContain("binary emitted malformed JSON");

    const invalid = generateWith(
      emitter(fixtures, "invalid", JSON.stringify({ version: 1 })),
      fixtures,
    );
    expect(invalid.status).toBe(1);
    expect(invalid.stderr).toContain("binary emitted an invalid schema bundle");

    if (process.platform !== "win32") {
      const signalled = resolve(fixtures, "signalled");
      writeFileSync(signalled, "#!/usr/bin/env node\nprocess.kill(process.pid, 'SIGTERM');\n");
      chmodSync(signalled, 0o755);
      const signalFailure = generateWith(signalled, fixtures);
      expect(signalFailure.status).toBe(1);
      expect(signalFailure.stderr).toContain("signal SIGTERM");
      expect(signalFailure.stderr).toContain("next: build the onetaskgraph binary");
    }
  } finally {
    rmSync(fixtures, { recursive: true, force: true });
  }
});

test("generator reports uncompileable roots and generated-file write failures", () => {
  const fixtures = mkdtempSync(resolve(tmpdir(), "onetaskgraph-generator-failures-"));
  const generated = resolve(fixtures, "generated");
  mkdirSync(generated);
  try {
    const invalidSchema = JSON.stringify({
      version: 1,
      roots: {
        Broken: {
          $schema: "https://json-schema.org/draft/2020-12/schema",
          $ref: "#/definitions/Missing",
        },
      },
      commands: ["schema"],
    });
    const compileFailure = generateWith(
      emitter(fixtures, "invalid-schema", invalidSchema),
      generated,
    );
    expect(compileFailure.status).toBe(1);
    expect(compileFailure.stderr).toContain("root Broken is not compilable JSON Schema");

    const validBundle = JSON.stringify({
      version: 1,
      roots: {
        Empty: { $schema: "https://json-schema.org/draft/2020-12/schema", type: "object" },
      },
      commands: ["schema"],
    });
    mkdirSync(resolve(generated, "models.ts"));
    const writeFailure = generateWith(emitter(fixtures, "valid", validBundle), generated);
    expect(writeFailure.status).toBe(1);
    expect(writeFailure.stderr).toContain("could not write");
    expect(writeFailure.stderr).toContain("next: check directory permissions");
  } finally {
    rmSync(fixtures, { recursive: true, force: true });
  }
});
