import { type ChildProcessByStdio, spawn } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import type { Readable } from "node:stream";

const packageRoot = resolve(import.meta.dir, "..");
// Bound every real-binary journey, including copy, if a subprocess genuinely hangs.
const HANG_TIMEOUT_MS = 60_000;

function report(status: number | null, signal: NodeJS.Signals | null, error?: Error) {
  process.stderr.write(
    `SDK test subprocess: ${JSON.stringify({
      schema_version: 1,
      status,
      signal,
      spawn_error: error?.message ?? null,
    })}\n`,
  );
}

async function run(args: string[], cwd = packageRoot) {
  let child: ChildProcessByStdio<null, Readable, Readable>;
  try {
    child = spawn("bun", ["test", `--timeout=${HANG_TIMEOUT_MS}`, ...args], {
      cwd,
      stdio: ["ignore", "pipe", "pipe"],
    });
  } catch (error) {
    const spawnError = error instanceof Error ? error : new Error(String(error));
    report(null, null, spawnError);
    return { status: null, signal: null, stdout: "", stderr: "", error: spawnError };
  }
  // Stream everything to CI; retain only the tail needed to check the runner's summaries.
  const TAIL_CHARACTERS = 64 * 1024;
  let stdout = "";
  let stderr = "";
  let spawnError: Error | undefined;
  child.stdout.on("data", (chunk: Buffer) => {
    process.stdout.write(chunk);
    stdout = (stdout + chunk.toString()).slice(-TAIL_CHARACTERS);
  });
  child.stderr.on("data", (chunk: Buffer) => {
    process.stderr.write(chunk);
    stderr = (stderr + chunk.toString()).slice(-TAIL_CHARACTERS);
  });
  child.on("error", (error: Error) => {
    spawnError = error;
  });
  const result = await new Promise<{ status: number | null; signal: NodeJS.Signals | null }>(
    (resolve) =>
      child.on("close", (status, signal) =>
        resolve({
          status: spawnError ? null : status,
          signal: spawnError ? null : (signal ?? null),
        }),
      ),
  );
  report(result.status, result.signal, spawnError);
  return { ...result, stdout, stderr, error: spawnError };
}

async function requireResult(
  label: string,
  expectedStatus: number,
  args: string[],
  cwd = packageRoot,
) {
  const result = await run(args, cwd);
  if (result.status !== expectedStatus) {
    throw new Error(
      `${label}: expected exit ${expectedStatus}, got ${result.status}\n${result.stdout}\n${result.stderr}`,
    );
  }
  return `${result.stdout}\n${result.stderr}`;
}

const args = process.argv.slice(2);
if (args.length > 0) {
  const result = await run(args);
  process.exitCode = result.status ?? 1;
} else {
  await requireResult("SDK suite", 0, []);

  const copies = await requireResult("copy journeys", 0, [
    "--test-name-pattern=copy drives the real binary|a document copy drives the real binary",
    "tests/client.test.ts",
  ]);
  if (!/2 pass/.test(copies) || !/0 fail/.test(copies)) {
    throw new Error(`copy journeys did not both pass under the shared timeout\n${copies}`);
  }

  const scratchRoot = process.env.ONEPIPELINE_NODE_SCRATCH_DIR ?? tmpdir();
  const fixtures = mkdtempSync(resolve(scratchRoot, "onetaskgraph-sdk-timeout-"));
  try {
    const delayed = resolve(fixtures, "delayed.test.ts");
    writeFileSync(
      delayed,
      `import { expect, test } from "bun:test";
import { type ChildProcessByStdio, spawn } from "node:child_process";
import type { Readable } from "node:stream";
test("a delayed subprocess succeeds", async () => {
  const child = spawn("bun", ["-e", "await Bun.sleep(5200); process.stdout.write('done')"]);
  let output = "";
  for await (const chunk of child.stdout) output += chunk;
  const code = await new Promise(resolve => child.on("close", resolve));
  expect(code).toBe(0);
  expect(output).toBe("done");
});
`,
    );
    const passed = await requireResult("delayed subprocess", 0, [delayed], fixtures);
    if (!/1 pass/.test(passed) || !/0 fail/.test(passed)) {
      throw new Error(`delayed subprocess did not pass\n${passed}`);
    }

    const stuck = resolve(fixtures, "stuck.test.ts");
    writeFileSync(
      stuck,
      `import { test } from "bun:test";
test("a stuck test is terminated", async () => {
  await new Promise(() => { setInterval(() => {}, 1000); });
});
`,
    );
    const refused = await requireResult("stuck test", 1, [stuck], fixtures);
    if (!refused.includes(`timed out after ${HANG_TIMEOUT_MS}ms`) || !/1 fail/.test(refused)) {
      throw new Error(`stuck test was not terminated by the shared timeout\n${refused}`);
    }
    process.stdout.write(
      "Shared timeout: delayed subprocess passed; stuck test timed out; both copy journeys passed.\n",
    );
  } finally {
    rmSync(fixtures, { recursive: true, force: true });
  }
}
