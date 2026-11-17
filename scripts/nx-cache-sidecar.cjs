// Ends the Nx remote-cache sidecar on its own, so the runner never has to cancel it.
//
// The sidecar (op-nx/github-cache/start-cache-server) is a background step that
// leaves only through its SIGTERM drain. Its `cancel:` teardown is a race in the
// runner (v2.338.2, BackgroundStepCoordinator.CancelWithGracePeriodAsync): it
// signals the step's process before it marks the step cancelled. A process that
// exits in between is reported "timed out after 0 minutes" as Failed, and a Failed
// background step fails the job. On Windows the Ctrl-C kills node at once, and that
// failed test-windows with every test green (run 44169712731).
//
// Two halves, one file:
//   - Loaded as a preload (`NODE_OPTIONS=--require ./scripts/nx-cache-sidecar.cjs`
//     on the sidecar step), it records that the sidecar started. When the stop file
//     appears it runs the sidecar's own SIGTERM drain inside the process, which then
//     exits 0, and it records the exit code. No OS signal is involved, so this
//     behaves the same on Linux, macOS and Windows.
//   - Run as `node scripts/nx-cache-sidecar.cjs stop` (an `if: always()` step
//     before `wait: nx-cache`), it writes the stop file and waits for that exit.
//     A drain that hangs is killed and reported, rather than left for `wait:` to
//     hold the job until its timeout. Exits 0 when the sidecar drained cleanly or
//     never started, 1 when it could not be stopped cleanly (stderr says why), and
//     2 on a usage error.
//
// Every file lives in "$RUNNER_TEMP/nx-cache-sidecar", which is the job's own.
"use strict";

const fs = require("node:fs");
const path = require("node:path");

const POLL_MS = 100;

// A process id fits a signed 32-bit integer on every platform; a drain bound past ten
// minutes is outside any job's teardown.
const MAX_PID = 2 ** 31 - 1;
const MAX_WAIT_MS = 600_000;

function positiveInteger(text, what, max) {
  const value = /^[1-9][0-9]*$/.test(text) ? Number(text) : Number.NaN;
  if (!(value <= max)) {
    throw new Error(
      `nx-cache-sidecar: ${what} is ${JSON.stringify(text)}, not an integer from 1 to ${max}; ${
        what === "the started file"
          ? "it is written only by the preload, so delete the nx-cache-sidecar directory under RUNNER_TEMP and re-run the job"
          : "unset it or set it to a whole number of milliseconds"
      }`,
    );
  }
  return value;
}

function directory() {
  const temp = process.env.RUNNER_TEMP;
  if (!temp) {
    throw new Error(
      "nx-cache-sidecar: RUNNER_TEMP is not set, so there is no job-owned directory for the stop handshake; run this inside an Actions job, or set RUNNER_TEMP to a scratch directory",
    );
  }
  return path.join(temp, "nx-cache-sidecar");
}

function preload() {
  // The sidecar's own children must not run this too.
  delete process.env.NODE_OPTIONS;
  const dir = directory();
  const unusable = (error, what) =>
    `nx-cache-sidecar: could not ${what} in ${dir} (${error.message}); the stop step cannot reach this sidecar without it, so check that RUNNER_TEMP is a writable directory and nothing else occupies that path, then re-run the job.\n`;
  try {
    fs.mkdirSync(dir, { recursive: true });
    for (const name of ["stop", "exited"]) fs.rmSync(path.join(dir, name), { force: true });
    fs.writeFileSync(path.join(dir, "started"), `${process.pid}\n`);
  } catch (error) {
    process.stderr.write(unusable(error, "set up the stop handshake"));
    process.exit(1);
  }
  process.on("exit", (code) => {
    try {
      fs.writeFileSync(path.join(dir, "exited"), `${code}\n`);
    } catch (error) {
      process.stderr.write(unusable(error, `record exit code ${code}`));
    }
  });
  const timer = setInterval(() => {
    if (!fs.existsSync(path.join(dir, "stop"))) return;
    clearInterval(timer);
    // The sidecar's drain is its SIGTERM listener; one that never registered has
    // nothing in flight to drain.
    if (process.listenerCount("SIGTERM") > 0) process.emit("SIGTERM", "SIGTERM");
    else process.exit(0);
  }, POLL_MS);
  timer.unref();
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function stop() {
  // How long a drain may take before it is killed. The override exists so the
  // hanging-drain journey does not wait the whole bound.
  const waitMs = positiveInteger(
    process.env.NX_CACHE_SIDECAR_STOP_WAIT_MS ?? "30000",
    "NX_CACHE_SIDECAR_STOP_WAIT_MS",
    MAX_WAIT_MS,
  );
  const dir = directory();
  const started = path.join(dir, "started");
  const exited = path.join(dir, "exited");
  // The sidecar step never ran (an earlier step failed), so there is nothing to end.
  if (!fs.existsSync(started)) return 0;
  const pid = positiveInteger(fs.readFileSync(started, "utf8").trim(), "the started file", MAX_PID);
  fs.writeFileSync(path.join(dir, "stop"), "");
  const deadline = Date.now() + waitMs;
  while (!fs.existsSync(exited)) {
    if (Date.now() > deadline) {
      let outcome = "so it was killed";
      try {
        process.kill(pid, "SIGKILL");
      } catch (error) {
        outcome = `and killing it failed (${error.code ?? error.message}), so the job's cancel step will end it`;
      }
      process.stderr.write(
        `nx-cache-sidecar: the sidecar (pid ${pid}) did not finish its drain within ${waitMs} ms of being asked to stop, ${outcome}. Read the sidecar step's log for the write it was waiting on.\n`,
      );
      return 1;
    }
    await sleep(POLL_MS);
  }
  const code = fs.readFileSync(exited, "utf8").trim();
  if (code !== "0") {
    process.stderr.write(
      `nx-cache-sidecar: the sidecar (pid ${pid}) exited with code ${code} while draining. Read the sidecar step's log for the error.\n`,
    );
    return 1;
  }
  return 0;
}

if (require.main === module) {
  if (process.argv[2] !== "stop") {
    process.stderr.write(
      "usage: node scripts/nx-cache-sidecar.cjs stop (or load it with --require on the sidecar step); exits 0 when the sidecar drained or never started, 1 when it could not be stopped cleanly, 2 on a usage error\n",
    );
    process.exitCode = 2;
  } else {
    stop().then(
      (code) => {
        process.exitCode = code;
      },
      (error) => {
        process.stderr.write(
          `${error.message}\nnx-cache-sidecar: the stop handshake under "$RUNNER_TEMP/nx-cache-sidecar" could not be read or written; check that directory exists and is writable by this job.\n`,
        );
        process.exitCode = 1;
      },
    );
  }
} else {
  preload();
}
