import { expect, test } from "bun:test";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const packageRoot = resolve(import.meta.dir, "..");
const wrapper = join(packageRoot, "scripts/test.ts");

test("the test CLI streams large output and coverage, and reports failure and spawn errors", async () => {
  const scratch = mkdtempSync(
    join(process.env.ONEPIPELINE_NODE_SCRATCH_DIR || tmpdir(), "sdk-runner-"),
  );
  try {
    const fixture = join(scratch, "output.test.ts");
    writeFileSync(join(scratch, "measured.ts"), "export const measured = () => 42;\n");
    const marker = "streamed-output-".repeat(140_000);
    writeFileSync(
      fixture,
      `import { expect, test } from "bun:test";
import { measured } from "./measured.ts";
test("large passing output", async () => {
  await new Promise<void>((resolve, reject) => process.stdout.write(${JSON.stringify(marker)}, (error) => error ? reject(error) : resolve()));
  expect(measured()).toBe(42);
});\n`,
    );
    async function capture(command: string[], env = process.env) {
      const child = Bun.spawn(command, {
        cwd: packageRoot,
        env,
        stdout: "pipe",
        stderr: "pipe",
      });
      const [stdout, stderr, status] = await Promise.all([
        new Response(child.stdout).text(),
        new Response(child.stderr).text(),
        child.exited,
      ]);
      return { stdout, stderr, status };
    }
    const drive = (args: string[], env = process.env) =>
      capture([process.execPath, wrapper, ...args], env);
    const direct = await capture([process.execPath, "test", "--coverage", fixture]);
    console.log(
      `SDK stream bytes: fixture=${Buffer.byteLength(marker)}, direct=${Buffer.byteLength(direct.stdout)}`,
    );
    expect(direct.status).toBe(0);
    expect(direct.stdout.includes(marker)).toBe(true);
    const passed = await drive(["--coverage", fixture]);
    console.log(`SDK stream bytes: wrapper=${Buffer.byteLength(passed.stdout)}`);
    expect(passed.status).toBe(0);
    expect(passed.stdout.includes(marker)).toBe(true);
    expect(passed.stderr).toContain("All files");
    expect(passed.stderr).toContain(
      'SDK test subprocess: {"schema_version":1,"status":0,"signal":null,"spawn_error":null}',
    );

    writeFileSync(
      fixture,
      'import { expect, test } from "bun:test"; test("failure", () => expect(1).toBe(2));\n',
    );
    const failed = await drive([fixture]);
    expect(failed.status).toBe(1);
    expect(failed.stderr).toContain(
      'SDK test subprocess: {"schema_version":1,"status":1,"signal":null,"spawn_error":null}',
    );
    expect(failed.stderr).toContain("1 fail");

    const noPath = Object.fromEntries(
      Object.entries(process.env).filter(([key]) => key.toUpperCase() !== "PATH"),
    );
    const missing = await drive([fixture], { ...noPath, PATH: scratch });
    expect(missing.status).toBe(1);
    expect(missing.stderr).toContain(
      'SDK test subprocess: {"schema_version":1,"status":null,"signal":null,"spawn_error":',
    );
    expect(missing.stderr).toMatch(/ENOENT|not found/i);
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
});
