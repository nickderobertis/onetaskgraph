import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";

const packageRoot = resolve(import.meta.dir, "..");
// Bound every real-binary journey, including copy, if a subprocess genuinely hangs.
const HANG_TIMEOUT_MS = 60_000;

function run(args: string[], cwd = packageRoot) {
  return spawnSync("bun", ["test", `--timeout=${HANG_TIMEOUT_MS}`, ...args], {
    cwd,
    encoding: "utf8",
  });
}

function requireResult(label: string, expectedStatus: number, args: string[], cwd = packageRoot) {
  const result = run(args, cwd);
  if (result.status !== expectedStatus) {
    throw new Error(
      `${label}: expected exit ${expectedStatus}, got ${result.status}\n${result.stdout}\n${result.stderr}`,
    );
  }
  return `${result.stdout}\n${result.stderr}`;
}

const args = process.argv.slice(2);
if (args.length > 0) {
  const result = run(args);
  process.stdout.write(result.stdout);
  process.stderr.write(result.stderr);
  process.exit(result.status ?? 1);
}

const suite = requireResult("SDK suite", 0, []);
process.stdout.write(suite);

const copies = requireResult("copy journeys", 0, [
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
import { spawn } from "node:child_process";
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
  const passed = requireResult("delayed subprocess", 0, [delayed], fixtures);
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
  const refused = requireResult("stuck test", 1, [stuck], fixtures);
  if (!refused.includes(`timed out after ${HANG_TIMEOUT_MS}ms`) || !/1 fail/.test(refused)) {
    throw new Error(`stuck test was not terminated by the shared timeout\n${refused}`);
  }
  process.stdout.write(
    "Shared timeout: delayed subprocess passed; stuck test timed out; both copy journeys passed.\n",
  );
} finally {
  rmSync(fixtures, { recursive: true, force: true });
}
