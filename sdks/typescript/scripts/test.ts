import { type ChildProcessByStdio, spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
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

async function run(args: string[], cwd = packageRoot, env: NodeJS.ProcessEnv = process.env) {
  let child: ChildProcessByStdio<null, Readable, Readable>;
  try {
    child = spawn("bun", ["test", `--timeout=${HANG_TIMEOUT_MS}`, ...args], {
      cwd,
      env,
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
  env: NodeJS.ProcessEnv = process.env,
) {
  const result = await run(args, cwd, env);
  if (result.status !== expectedStatus || result.signal !== null) {
    throw new Error(
      `${label}: expected exit ${expectedStatus}, got ${result.status} (signal ${result.signal}); read the runner's output below for the failing test, then re-run \`bun scripts/test.ts ${args.join(" ")}\` from sdks/typescript\n${result.stdout}\n${result.stderr}`,
    );
  }
  return `${result.stdout}\n${result.stderr}`;
}

// One test case of a fixture as the runner itself reported it in its JUnit report, rather than
// as its human summary reads: its name and the type of the failure it recorded, if any.
type ReportedCase = { name: string; failure: string | null };

function reportedCases(report: string): ReportedCase[] {
  const xml = readFileSync(report, "utf8");
  return [...xml.matchAll(/<testcase\b([^>]*?)(?:\/>|>([\s\S]*?)<\/testcase>)/g)].map(
    ([, attributes, body]) => ({
      name: unescapeXml(/\bname="([^"]*)"/.exec(attributes)?.[1] ?? ""),
      failure: body?.match(/<failure\b[^>]*?\btype="([^"]*)"/)?.[1] ?? null,
    }),
  );
}

function unescapeXml(text: string) {
  return text
    .replaceAll("&quot;", '"')
    .replaceAll("&apos;", "'")
    .replaceAll("&lt;", "<")
    .replaceAll("&gt;", ">")
    .replaceAll("&amp;", "&");
}

// A fixture's run: under the shared timeout, reporting to a JUnit file the assertions read, and
// with GITHUB_ACTIONS cleared, so the stuck fixture's expected timeout is not written into the
// CI run as an error annotation — it read as this suite failing while the suite passed.
async function runFixture(
  label: string,
  expectedStatus: number,
  fixture: string,
  fixtures: string,
) {
  const report = resolve(fixtures, `${label.replaceAll(" ", "-")}.junit.xml`);
  const output = await requireResult(
    label,
    expectedStatus,
    ["--reporter=junit", `--reporter-outfile=${report}`, fixture],
    fixtures,
    {
      ...process.env,
      GITHUB_ACTIONS: "",
      STARTED_AT: resolve(fixtures, `${label.replaceAll(" ", "-")}.started`),
    },
  );
  if (!existsSync(report)) {
    throw new Error(
      `${label}: the runner wrote no JUnit report at ${report}; check that this bun (\`bun --version\`) still accepts --reporter=junit --reporter-outfile, and read its output below\n${output}`,
    );
  }
  return { cases: reportedCases(report), output };
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
    const passed = await runFixture("delayed subprocess", 0, delayed, fixtures);
    const expectedPass = [{ name: "a delayed subprocess succeeds", failure: null }];
    if (JSON.stringify(passed.cases) !== JSON.stringify(expectedPass)) {
      throw new Error(
        `delayed subprocess did not pass: the runner reported ${JSON.stringify(passed.cases)}; a TimeoutError means the shared timeout (${HANG_TIMEOUT_MS} ms) no longer covers a 5.2 s subprocess, anything else is in the output below\n${passed.output}`,
      );
    }

    // The stuck test records that its body began, then never settles; what ends it has to be the
    // runner's own timeout, reported as that test's TimeoutError, with the runner exiting by
    // itself (status 1, no signal) rather than being killed.
    const stuck = resolve(fixtures, "stuck.test.ts");
    writeFileSync(
      stuck,
      `import { test } from "bun:test";
import { writeFileSync } from "node:fs";
test("a stuck test is terminated", async () => {
  writeFileSync(process.env.STARTED_AT ?? "", "started");
  await new Promise(() => { setInterval(() => {}, 1000); });
});
`,
    );
    const refused = await runFixture("stuck test", 1, stuck, fixtures);
    if (!existsSync(resolve(fixtures, "stuck-test.started"))) {
      throw new Error(
        `the stuck test's body never began, so nothing was stuck; read the runner's output below for why the fixture did not load or run\n${refused.output}`,
      );
    }
    const expectedTimeout = [{ name: "a stuck test is terminated", failure: "TimeoutError" }];
    if (JSON.stringify(refused.cases) !== JSON.stringify(expectedTimeout)) {
      throw new Error(
        `stuck test was not terminated by the shared timeout: the runner reported ${JSON.stringify(refused.cases)} where it owes one TimeoutError; check that \`--timeout=${HANG_TIMEOUT_MS}\` still reaches \`bun test\` in run(), and read the output below\n${refused.output}`,
      );
    }
    process.stdout.write(
      "Shared timeout: delayed subprocess passed; stuck test timed out; both copy journeys passed.\n",
    );
  } finally {
    rmSync(fixtures, { recursive: true, force: true });
  }
}
