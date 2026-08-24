import { expect, test } from "bun:test";
import { appendFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { spawnSync } from "node:child_process";

const packageRoot = resolve(import.meta.dir, "..");
const binary = resolve(packageRoot, "../../target/debug/onetaskgraph");

test("generation, clean check, stale check, and invalid arguments use the real binary", () => {
  const generated = mkdtempSync(resolve(tmpdir(), "onetaskgraph-generated-"));
  const run = (...args: string[]) =>
    spawnSync("bun", ["scripts/generate.ts", ...args], {
      cwd: packageRoot,
      encoding: "utf8",
      env: {
        ...process.env,
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
