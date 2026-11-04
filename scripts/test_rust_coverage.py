"""Exercise coverage progress and refusal through real Cargo and nextest processes."""

import asyncio
import json
import os
import socket
from pathlib import Path
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]


class CoverageProgressTest(unittest.TestCase):
    def test_progress_precedes_completion_and_failure_stays_nonzero(self):
        asyncio.run(self.exercise())

    def test_native_progress_precedes_completion_and_failure_stays_nonzero(self):
        asyncio.run(self.exercise(native=True))

    async def exercise(self, native=False):
        native = native or os.name == "nt"
        with tempfile.TemporaryDirectory(
            prefix="coverage-progress-", dir=os.environ.get("ONEPIPELINE_NODE_SCRATCH_DIR")
        ) as scratch:
            tree = Path(scratch)
            (tree / "rust-toolchain.toml").write_text(
                (ROOT / "rust-toolchain.toml").read_text(encoding="utf-8"), encoding="utf-8"
            )
            (tree / "src").mkdir()
            (tree / "Cargo.toml").write_text(
                '[package]\nname = "coverage-progress"\nversion = "0.1.0"\nedition = "2024"\n',
                encoding="utf-8",
            )
            (tree / "src/lib.rs").write_text(
                '''#[test]
fn waits_for_release() {
    let gate = std::env::var("COVERAGE_PROGRESS_GATE").unwrap();
    while !std::path::Path::new(&gate).exists() {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(std::env::var_os("COVERAGE_PROGRESS_FAIL").is_none(), "induced failure");
}
''',
                encoding="utf-8",
            )
            lock = await asyncio.create_subprocess_exec(
                "cargo", "generate-lockfile", "--offline", cwd=tree,
                stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.STDOUT,
            )
            diagnostic = (await lock.communicate())[0].decode("utf-8", errors="replace")
            self.assertEqual(lock.returncode, 0, diagnostic)
            command = ["bash", str(ROOT / "scripts/rust-coverage.sh"), "coverage-progress"]
            if native:
                # Windows does not measure coverage, but still exercises nextest's
                # progress stream and failure propagation without a shell or WSL.
                command = ["cargo", "nextest", "run", "--locked"]
            for fails in (False, True):
                gate = tree / f"release-{fails}"
                environment = dict(os.environ, COVERAGE_PROGRESS_GATE=str(gate))
                if fails:
                    environment["COVERAGE_PROGRESS_FAIL"] = "1"
                environment.update(
                    NEXTEST_STATUS_LEVEL="all",
                    NEXTEST_EXPERIMENTAL_LIBTEST_JSON="1",
                    NEXTEST_MESSAGE_FORMAT="libtest-json-plus",
                )
                # Nx supplies a socket for subprocess output. It cannot be reopened via
                # /dev/stderr, although writing the inherited descriptor works.
                writer = None
                if native:
                    process = await asyncio.create_subprocess_exec(
                        *command, cwd=tree, env=environment,
                        stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.STDOUT,
                    )
                    reader = process.stdout
                else:
                    parent, child = socket.socketpair()
                    parent.setblocking(False)
                    reader, writer = await asyncio.open_connection(sock=parent)
                    process = await asyncio.create_subprocess_exec(
                        *command, cwd=tree, env=environment, stdout=child,
                        stderr=asyncio.subprocess.STDOUT,
                    )
                    child.close()
                output = ""
                try:
                    async with asyncio.timeout(120):
                        while line := await reader.readline():
                            output += line.decode("utf-8", errors="replace")
                            self.assertNotIn("tee:", output)
                            try:
                                event = json.loads(line)
                            except ValueError:
                                continue
                            if isinstance(event, dict) and event.get("type") == "test" and event.get("event") == "started":
                                self.assertIsNone(process.returncode, output)
                                self.assertFalse(gate.exists())
                                gate.touch()
                                break
                        else:
                            self.fail(f"no test start was streamed: {output}")
                        output += (await reader.read()).decode("utf-8", errors="replace")
                        await process.wait()
                finally:
                    gate.touch(exist_ok=True)
                    if process.returncode is None:
                        process.terminate()
                        await process.wait()
                    if writer is not None:
                        writer.close()
                        await writer.wait_closed()
                if fails:
                    self.assertNotEqual(process.returncode, 0, output)
                else:
                    self.assertEqual(process.returncode, 0, output)
                self.assertIn("FAIL" if fails else "PASS", output)
                if fails:
                    self.assertIn("induced failure", output)
                    if not native:
                        self.assertIn("tests failed under instrumentation", output)


if __name__ == "__main__":
    unittest.main()
