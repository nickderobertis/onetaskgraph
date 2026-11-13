"""The profile's accounting, driven through the same entry points the command uses.

Run: python3 -m unittest spike/ci-profile/test_ciprofile.py
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import measurement_only  # noqa: E402
import profile as prof  # noqa: E402
import replay  # noqa: E402
import sampler  # noqa: E402

JOB = {
    "id": 1, "name": "test-macos", "conclusion": "success",
    "started_at": "2026-11-12T04:00:00Z", "completed_at": "2026-11-12T04:10:00Z",
    "steps": [
        {"name": "Set up job", "started_at": "2026-11-12T04:00:00Z", "completed_at": "2026-11-12T04:00:10Z", "conclusion": "success"},
        {"name": "Run actions-rust-lang/setup-rust-toolchain@v1", "started_at": "2026-11-12T04:00:10Z", "completed_at": "2026-11-12T04:01:00Z", "conclusion": "success"},
        {"name": "Run actions/cache/restore@v6", "started_at": "2026-11-12T04:01:00Z", "completed_at": "2026-11-12T04:01:05Z", "conclusion": "success"},
        {"name": "Test the affected projects", "started_at": "2026-11-12T04:01:05Z", "completed_at": "2026-11-12T04:09:05Z", "conclusion": "success"},
        {"name": "Post Run astral-sh/setup-uv@v7", "started_at": "2026-11-12T04:09:05Z", "completed_at": "2026-11-12T04:09:35Z", "conclusion": "success"},
        {"name": "Complete job", "started_at": "2026-11-12T04:09:35Z", "completed_at": "2026-11-12T04:10:00Z", "conclusion": "success"},
    ],
}
LOG = "\n".join([
    "2026-11-12T04:00:10.0000000Z ##[group]Run actions-rust-lang/setup-rust-toolchain@v1",
    "2026-11-12T04:00:20.0000000Z ##[group]Run Swatinem/rust-cache@abc",
    "2026-11-12T04:00:25.0000000Z Cache Size: ~1234 MB",
    "2026-11-12T04:00:50.0000000Z Restored from cache key \"v0-rust-test-macos\"",
    "2026-11-12T04:01:00.0000000Z ##[group]Run actions/cache/restore@v6",
    "2026-11-12T04:01:05.0000000Z ##[group]Run just ci-test-macos",
    "2026-11-12T04:01:06.0000000Z [test,budgets] 🔁 > nx run a-crate:test  [remote cache]",
    "2026-11-12T04:01:07.0000000Z [test,budgets] > nx run b-crate:test",
    "2026-11-12T04:04:07.0000000Z [test,budgets] b-crate: running 3 tests",
    "2026-11-12T04:05:07.0000000Z [test,budgets] b-crate: test result: ok. 3 passed; finished in 60.00s",
    "2026-11-12T04:09:05.0000000Z ##[group]Run node scripts/nx-remote-cache.mjs report",
    "2026-11-12T04:09:06.0000000Z Nx cache: stored 2 entries, 100 bytes; replayed 1 entries",
])
EXECUTIONS = {"tasks": [
    {"task": "a-crate:test", "result": "remote_hit", "started_at": "2026-11-12T04:01:06.000Z", "finished_at": "2026-11-12T04:01:07.000Z"},
    {"task": "b-crate:test", "result": "executed", "started_at": "2026-11-12T04:01:07.000Z", "finished_at": "2026-11-12T04:05:07.000Z"},
    {"task": "c-crate:test", "result": "executed", "started_at": "2026-11-12T04:03:07.000Z", "finished_at": "2026-11-12T04:05:07.000Z"},
]}


class ProfileTests(unittest.TestCase):
    def test_every_second_of_the_job_is_counted_once(self):
        p = prof.profile_job(JOB, LOG, EXECUTIONS)
        self.assertAlmostEqual(sum(p.buckets.values()), p.wall, places=6)
        self.assertAlmostEqual(p.wall, 600)
        # Two tasks overlap 04:03:07–04:05:07: each gets half of those 120 s.
        self.assertAlmostEqual(p.task_share["b-crate:test"], 120 + 60, places=3)
        self.assertAlmostEqual(p.task_share["c-crate:test"], 60, places=3)
        self.assertAlmostEqual(p.task_share["a-crate:test"], 1, places=3)
        # b compiles until its first test starts (04:04:07); c printed no test start, so it is test run.
        self.assertAlmostEqual(p.buckets["compile-link(plain)"], 120 + 30, places=3)
        self.assertEqual(p.rust_cache["size"], "1234 MB")
        self.assertEqual(p.stored, (2, 100, 1))
        self.assertGreater(p.buckets["cache-save"], 29)
        self.assertAlmostEqual(p.buckets["nx-planning"], 1 + 238, places=3)  # before the first task, and 04:05:07 → 04:09:05

    def test_the_log_alone_still_places_tasks(self):
        p = prof.profile_job(JOB, LOG, None)
        names = {t.name: t.result for t in p.tasks}
        self.assertEqual(names, {"a-crate:test": "remote_hit", "b-crate:test": "executed"})
        self.assertAlmostEqual(sum(p.buckets.values()), p.wall, places=6)

    def test_the_sampler_divides_a_window_and_calls_an_idle_tick_a_wait(self):
        ticks = [
            {"kind": "tick", "t": 10.0, "dt": 1.0, "leaves": {"compile": 2, "link": 1}, "cpu": {"compile": 2.0, "link": 1.0}},
            {"kind": "tick", "t": 11.0, "dt": 1.0, "leaves": {"test-binary": 1}, "cpu": {"test-binary": 0.01}},
            {"kind": "exit", "pid": 4, "name": "git", "category": "git", "start": 10.2, "end": 10.7, "cpu": 0.1},
        ]
        out = prof.load_sampler([json.dumps(t) for t in ticks], (0, 100))
        self.assertEqual(out["buckets"]["compile"], 0.5)
        self.assertEqual(out["buckets"]["link"], 0.5)
        self.assertEqual(out["buckets"]["wait:network-or-sleep(test)"], 1.0)
        self.assertEqual(out["exits"]["git"], {"count": 1, "seconds": 0.5})


class RecorderTests(unittest.TestCase):
    def test_the_runner_keeps_the_exit_status_and_records_each_test(self):
        tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, tmp)
        fake = tmp / "fake_bin-0123456789abcdef.py"
        fake.write_text(
            "import json,sys\n"
            "assert '--format=json' in sys.argv and '-q' not in sys.argv\n"
            "print(json.dumps({'type':'suite','event':'started','test_count':1}))\n"
            "print(json.dumps({'type':'test','event':'failed','name':'t::slow','exec_time':1.5,'stdout':'boom'}))\n"
            "print(json.dumps({'type':'suite','event':'failed','passed':0,'failed':1,'exec_time':1.5}))\n"
            "sys.exit(101)\n")
        record = tmp / "tests.jsonl"
        proc = subprocess.run([sys.executable, str(HERE / "testrunner.py"), sys.executable, str(fake), "-q"],
                              capture_output=True, text=True, env=dict(os.environ, SPIKE_TEST_RECORD=str(record)))
        self.assertEqual(proc.returncode, 101)
        self.assertIn("boom", proc.stdout)
        self.assertIn("test result: failed", proc.stdout)
        events = [json.loads(x) for x in record.read_text().splitlines()]
        self.assertEqual([e["exec_time"] for e in events if e.get("type") == "test"], [1.5])


class ReplayTests(unittest.TestCase):
    def test_each_option_replays_only_a_task_its_rule_takes_every_match_from(self):
        task = "onetaskgraph-e2e:test"
        self.assertTrue(replay.options(task, ["biome.json", "AGENTS.md"], {"biome.json", "AGENTS.md"})["non-semantic-globals-out-of-rust-keys"])
        self.assertFalse(replay.options(task, ["biome.json", "crates/onetaskgraph-core/src/lib.rs"], set())["non-semantic-globals-out-of-rust-keys"])
        self.assertTrue(replay.options(task, ["crates/onetaskgraph-core/tests/engine.rs"], set())["dependents-skip-dependency-tests"])
        self.assertFalse(replay.options(task, ["crates/onetaskgraph-e2e/tests/e2e/main.rs"], set())["dependents-skip-dependency-tests"])
        self.assertTrue(replay.options("scripts:distribution-test", [], {"AGENTS.md", ".github/workflows/ci.yml"})["distribution-only-when-publishable-changes"])
        self.assertFalse(replay.options("scripts:distribution-test", [], {"crates/onetaskgraph-core/src/lib.rs"})["distribution-only-when-publishable-changes"])
        self.assertEqual(replay.most_specific({"dep-src", "own-tests", "scripts"}), "own-tests")


class SamplerTests(unittest.TestCase):
    def test_processes_are_classified_by_what_the_job_spends_on(self):
        cases = [("rustc", [], "compile"), ("ld64", [], "link"), ("link.exe", [], "link"),
                 ("onetaskgraph.exe", [], "cli-under-test"), ("e2e-0123456789abcdef", [], "test-binary"),
                 ("node", ["node", "/w/node_modules/nx/bin/nx.js", "run"], "nx"), ("node", ["node", "x.test.mjs"], "node"),
                 ("python3.13", [], "python"), ("git", [], "git"), ("bun", [], "bun"), ("gtar", [], "archive")]
        for name, cmd, want in cases:
            self.assertEqual(sampler.classify(name, cmd), want, name)


class MeasurementOnlyTests(unittest.TestCase):
    def test_todays_workflow_writes_no_cache_and_records_every_test_job(self):
        text = measurement_only.apply((HERE.parents[1] / ".github/workflows/ci.yml").read_text(encoding="utf-8"), "x,y")
        self.assertNotIn("save-cache: ${{", text)
        self.assertNotRegex(text, r"cache-save-if: (?!false)")
        self.assertNotRegex(text, r"CACHE_READ_ONLY: (?!'1')")
        self.assertNotIn("nx-cache-entries-", text)
        for job in ("test-linux", "test-macos", "test-windows"):
            self.assertIn(f"name: spike-profile-{job}", text)
            self.assertIn(f"just ci-{job} --exclude=x,y || status=$?", text)


if __name__ == "__main__":
    unittest.main()
