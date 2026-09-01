import { expect, test } from "bun:test";
import {
  appendFileSync,
  chmodSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";

const packageRoot = resolve(import.meta.dir, "..");
const binary = resolve(packageRoot, "../../target/debug/onetaskgraph");

function executable(directory: string, name: string, body: string) {
  const windows = process.platform === "win32";
  const program = resolve(directory, `${name}.js`);
  const path = windows ? resolve(directory, `${name}.cmd`) : program;
  writeFileSync(program, `#!/usr/bin/env node\n${body}\n`);
  if (windows) writeFileSync(path, `@echo off\r\nnode "%~dp0${name}.js"\r\n`);
  else chmodSync(path, 0o755);
  return path;
}

function emitter(directory: string, name: string, output: string) {
  return executable(directory, name, `process.stdout.write(${JSON.stringify(output)});`);
}

function expectExited(result: ReturnType<typeof spawnSync>) {
  expect(
    result.status,
    `generator was killed by ${result.signal ?? "an unknown signal"}`,
  ).not.toBeNull();
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
    expectExited(generatedResult);
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
    expectExited(unavailable);
    expect(unavailable.status).toBe(1);
    expect(unavailable.stderr).toContain("could not emit the SDK contract");
    const emptyBinary = spawnSync("bun", ["scripts/generate.ts"], {
      cwd: packageRoot,
      encoding: "utf8",
      env: { ...process.env, ONETASKGRAPH_BIN: "" },
    });
    expectExited(emptyBinary);
    expect(emptyBinary.status).toBe(1);
    expect(emptyBinary.stderr).toContain("ONETASKGRAPH_BIN must be a non-empty executable path");
  } finally {
    rmSync(generated, { recursive: true, force: true });
  }
}, 30_000);

test("a description in several paragraphs generates without trailing whitespace", () => {
  // `json-schema-to-typescript` renders a paragraph break inside a JSDoc block as a line
  // that is exactly `" * "`. Committed, that is what `git diff --check` reports against
  // every change adding a field documented in more than one paragraph — and the contract's
  // fields really are documented that way, so this is a property of the generator rather
  // than of any one field. A generator without the fix emits `" * "` here and fails.
  const fixtures = mkdtempSync(resolve(tmpdir(), "onetaskgraph-generator-whitespace-"));
  const generated = mkdtempSync(resolve(tmpdir(), "onetaskgraph-generated-"));
  try {
    const bundle = {
      version: 1,
      roots: {
        Thing: {
          $schema: "https://json-schema.org/draft/2020-12/schema",
          type: "object",
          properties: {
            note: {
              description: "The first paragraph.\n\nThe second, after a blank line.",
              type: "string",
            },
          },
        },
      },
      commands: ["thing list"],
    };
    const result = generateWith(
      emitter(fixtures, "paragraphs", JSON.stringify(bundle)),
      generated,
    );
    expectExited(result);
    expect(result.status, result.stderr).toBe(0);
    const models = readFileSync(resolve(generated, "models.ts"), "utf8");
    expect(models).toContain("The second, after a blank line.");
    expect(models.split("\n").filter((line) => /[ \t]+$/.test(line))).toEqual([]);
  } finally {
    rmSync(fixtures, { recursive: true, force: true });
    rmSync(generated, { recursive: true, force: true });
  }
}, 30_000);

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
    expectExited(unsafe);
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
    expectExited(missingDestination);
    expect(missingDestination.status).toBe(1);
    expect(missingDestination.stderr).toContain("could not resolve ONETASKGRAPH_GENERATED_DIR");
    expect(missingDestination.stderr).toContain("next: create the directory");

    const nonJson = generateWith(emitter(fixtures, "non-json", "not JSON"), fixtures);
    expectExited(nonJson);
    expect(nonJson.status).toBe(1);
    expect(nonJson.stderr).toContain("binary emitted malformed JSON");

    const invalid = generateWith(
      emitter(fixtures, "invalid", JSON.stringify({ version: 1 })),
      fixtures,
    );
    expectExited(invalid);
    expect(invalid.status).toBe(1);
    expect(invalid.stderr).toContain("binary emitted an invalid schema bundle");

    const failed = generateWith(
      executable(
        fixtures,
        "failed",
        'process.stderr.write("fixture unavailable"); process.exit(7);',
      ),
      fixtures,
    );
    expectExited(failed);
    expect(failed.status).toBe(1);
    expect(failed.stderr).toContain("fixture unavailable");

    const invalidBundles: ReadonlyArray<readonly [string, unknown]> = [
      ["version", { version: "1", roots: {}, commands: [] }],
      ["negative-version", { version: -1, roots: {}, commands: [] }],
      ["roots", { version: 1, roots: [], commands: [] }],
      [
        "unsafe-root",
        {
          version: 1,
          roots: { "not-safe": { $schema: "https://json-schema.org/draft/2020-12/schema" } },
          commands: [],
        },
      ],
      ["schema", { version: 1, roots: { Broken: { type: "object" } }, commands: [] }],
      ["commands", { version: 1, roots: {}, commands: [1] }],
      ["duplicate-commands", { version: 1, roots: {}, commands: ["schema", "schema"] }],
    ];
    for (const [name, bundle] of invalidBundles) {
      const result = generateWith(emitter(fixtures, name, JSON.stringify(bundle)), fixtures);
      expectExited(result);
      expect(result.status).toBe(1);
      expect(result.stderr).toContain("binary emitted an invalid schema bundle");
    }

    if (process.platform !== "win32") {
      const signalled = resolve(fixtures, "signalled");
      writeFileSync(signalled, "#!/usr/bin/env node\nprocess.kill(process.pid, 'SIGTERM');\n");
      chmodSync(signalled, 0o755);
      const signalFailure = generateWith(signalled, fixtures);
      expectExited(signalFailure);
      expect(signalFailure.status).toBe(1);
      expect(signalFailure.stderr).toContain("signal SIGTERM");
      expect(signalFailure.stderr).toContain("next: build the onetaskgraph binary");
    }
  } finally {
    rmSync(fixtures, { recursive: true, force: true });
  }
}, 30_000);

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
    expectExited(compileFailure);
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
    expectExited(writeFailure);
    expect(writeFailure.status).toBe(1);
    expect(writeFailure.stderr).toContain("could not write");
    expect(writeFailure.stderr).toContain("next: check directory permissions");
  } finally {
    rmSync(fixtures, { recursive: true, force: true });
  }
}, 30_000);
