"""Each record the harness shares with another program, held to that program; and the command paths
`test_ciprofile.py` leaves, driven through the real entry points with a stand-in `gh` on PATH.
"""

from __future__ import annotations

import asyncio
import dataclasses
import io
import json
import re
import shutil
import subprocess
import sys
import textwrap
import unittest
import unittest.mock
import zipfile
from pathlib import Path
from typing import Any

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

# E402: the modules under test sit beside this file and are found through the sys.path entry above.
import ghcache  # noqa: E402
import jobprofile as prof  # noqa: E402
import measurement_only  # noqa: E402
import models  # noqa: E402
import replay  # noqa: E402
import sampler  # noqa: E402
from test_ciprofile import (  # noqa: E402
    EXECUTIONS,
    JOB,
    LOG,
    RUN,
    RUN_RECORD,
    STEPS,
    command,
    fake_gh,
    fixture_cache,
    scratch,
)

REPO = HERE.parents[1]


class ProducerContractTests(unittest.TestCase):
    def test_the_nx_results_and_fields_are_the_ones_the_cache_report_writes(self):
        source = (REPO / "scripts/nx-remote-cache.mjs").read_text()
        expression = re.search(r"const result =(.*?);", source, re.S)
        pushed = re.search(r"tasks\.push\(\{(.*?)\}\);", source, re.S)
        assert expression is not None and pushed is not None, "nx-remote-cache.mjs no longer writes its task records"
        written = set(re.findall(r"'(\w+)'", expression.group(1))) - {"local-cache", "remote-cache"}
        self.assertEqual(written, {r.value for r in models.TaskResult} | {"failed"})
        for field in ("task", "result", "started_at", "finished_at"):
            self.assertRegex(pushed.group(1), rf"\b{field}\b")

    def test_every_event_the_runner_kept_from_real_libtest_binaries_parses(self):
        recorded = (HERE / "data/libtest/run-43378506851-macos-events.jsonl").read_text().splitlines()
        events = [models.LibtestEvent.parse(json.loads(line)) for line in recorded]
        self.assertIn(models.LibtestOutcome.TIMEOUT, {e.outcome for e in events})
        self.assertTrue(
            all(
                e.exec_time is not None
                for e in events
                if e.kind is models.LibtestKind.TEST
                and e.outcome in (models.LibtestOutcome.OK, models.LibtestOutcome.FAILED)
            )
        )

    def test_a_finished_event_without_its_timing_or_counts_is_refused(self):
        with self.assertRaisesRegex(models.Invalid, "carries no exec_time"):
            models.LibtestEvent.parse({"type": "test", "event": "ok", "name": "t"}, "b")
        with self.assertRaisesRegex(models.Invalid, "carries no ignored, filtered_out"):
            models.LibtestEvent.parse({"type": "suite", "event": "ok", "passed": 1, "failed": 0, "exec_time": 1.0}, "b")

    def test_a_libtest_event_libtest_never_emits_is_refused(self):
        with self.assertRaisesRegex(models.Invalid, "emits no suite ignored event"):
            models.LibtestEvent.parse({"type": "suite", "event": "ignored"}, "b")
        self.assertEqual(
            models.LibtestEvent.parse({"type": "test", "event": "ignored", "name": "t"}, "b").outcome,
            models.LibtestOutcome.IGNORED,
        )

    def test_a_process_seen_once_within_a_tick_reads_as_a_zero_lifetime(self):
        seen_once = models.sampler_record({"kind": "exit", "category": "git", "start": 10.3, "end": 10.0, "cpu": 0.0})
        self.assertEqual(seen_once, models.Exit(models.Category.GIT, 10.3, 10.3, 0.0))
        with self.assertRaisesRegex(models.Invalid, "ends before it starts"):
            models.sampler_record({"kind": "exit", "category": "git", "start": 20.0, "end": 10.0, "cpu": 0.0})

    def test_sampler_records_read_back_as_they_were_written(self):
        tick = models.Tick(5.0, 1.0, {models.Category.COMPILE: 2}, {models.Category.COMPILE: 1.5})
        gone = models.Exit(models.Category.GIT, 1.0, 2.5, 0.25)
        for record in (tick, gone):
            self.assertEqual(models.sampler_record(json.loads(json.dumps(record.to_json()))), record)

    @unittest.skipUnless(shutil.which("rustc"), "compiles a libtest binary with rustc")
    def test_a_real_libtest_binary_is_recorded_through_the_runner(self):
        root = scratch(self)
        source = root / "probe.rs"
        source.write_text(
            "#[test] fn passes() { std::thread::sleep(std::time::Duration::from_millis(30)); }\n"
            "#[test] #[ignore] fn skipped() {}\n"
        )
        binary = root / "probe-0123456789abcdef"
        subprocess.run(["rustc", "--test", "-o", str(binary), str(source)], check=True, capture_output=True)
        record = root / "spike" / "tests.jsonl"
        record.parent.mkdir()
        proc = command("testrunner.py", str(binary), "-q", env={"SPIKE_TEST_RECORD": str(record)})
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("test result: ok. 1 passed; 0 failed; 1 ignored", proc.stdout)
        events = [models.LibtestEvent.parse(json.loads(x)) for x in record.read_text().splitlines()]
        passed = [e for e in events if e.kind is models.LibtestKind.TEST and e.outcome is models.LibtestOutcome.OK]
        self.assertEqual([e.name for e in passed], ["passes"])
        self.assertGreaterEqual(passed[0].exec_time or 0.0, 0.03)

    def test_the_history_the_command_writes_is_the_history_the_replay_reads(self):
        root = scratch(self)
        cache = fixture_cache(root)
        listing = cache / "lists/event_pull_request_created_>_2026-11-01.1.json"
        listing.parent.mkdir(parents=True)
        listing.write_text(json.dumps({"workflow_runs": [RUN_RECORD]}))
        (root / "prs.json").write_text(json.dumps([{"pr": 9, "head": "b" * 40, "branch": "topic"}]))
        env = {"CIPROFILE_CACHE": str(cache), "CIPROFILE_OFFLINE": "1"}
        self.assertEqual(
            command(
                "ciprofile.py", "history", str(root / "prs.json"), "--out", str(root / "h.json"), env=env
            ).returncode,
            0,
        )
        (root / "after").mkdir()
        (root / "after/b-crate__test.json").write_text(json.dumps({"files": ["crates/b-crate/src/lib.rs"]}))
        (root / "files.json").write_text(json.dumps([{"pr": 9, "files": ["crates/b-crate/src/lib.rs"]}]))
        proc = command(
            "replay.py",
            str(root / "h.json"),
            str(root / "files.json"),
            str(root / "after"),
            str(root / "after"),
            "--json",
            str(root / "r.json"),
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        rows = json.loads((root / "r.json").read_text())["rows"]["9"]["test-macos"]
        self.assertEqual(rows["b-crate:test"]["why"], "own-src")


class CommandPathTests(unittest.TestCase):
    def offline(self, root: Path) -> dict:
        return {"CIPROFILE_CACHE": str(fixture_cache(root)), "CIPROFILE_OFFLINE": "1"}

    def test_profile_writes_the_json_it_printed(self):
        root = scratch(self)
        env = self.offline(root)
        proc = command("ciprofile.py", "profile", str(RUN), "--jobs", "test-macos", "--json", env=env)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        summary = json.loads((root / f"cache/profile-{RUN}.json").read_text())
        job = summary["jobs"]["test-macos"]
        self.assertEqual(job["wall_s"], 600.0)
        self.assertAlmostEqual(sum(job["buckets_s"].values()), 600.0, delta=0.5)
        self.assertEqual(job["sampler"]["exits"]["cli-under-test"], {"count": 1, "lifetime": 1.0, "cpu": 0.1})

    def test_a_run_the_cache_does_not_hold_offline_exits_4(self):
        root = scratch(self)
        proc = command("ciprofile.py", "profile", "123", env=self.offline(root))
        self.assertEqual(proc.returncode, 4)
        self.assertIn("Offline", proc.stderr)

    def test_a_malformed_cached_record_exits_4_naming_the_field(self):
        root = scratch(self)
        env = self.offline(root)
        (Path(env["CIPROFILE_CACHE"]) / f"runs/{RUN}/jobs.json").write_text(json.dumps({"jobs": [{**JOB, "id": "x"}]}))
        proc = command("ciprofile.py", "profile", str(RUN), env=env)
        self.assertEqual(proc.returncode, 4)
        self.assertIn("'id' is str", proc.stderr)

    def test_durations_count_only_successful_test_jobs_of_non_release_runs(self):
        root = scratch(self)
        query = (
            "actions/workflows/ci.yml/runs?event=pull_request&created=2026-11-01T00:00:00Z..2026-11-02T00:00:00Z"
            "&per_page=100&page=1"
        )
        linux = {**JOB, "name": "test-linux"}
        env = fake_gh(
            root,
            remaining=4000,
            responses={
                query: {
                    "workflow_runs": [
                        RUN_RECORD,
                        {**RUN_RECORD, "id": 78, "head_branch": "release-plz-2026-11-01"},
                        {**RUN_RECORD, "id": 79, "conclusion": "cancelled"},
                    ]
                },
                f"actions/runs/{RUN}/jobs?per_page=100&page=1": {
                    "total_count": 2,
                    "jobs": [linux, {**linux, "id": 6, "name": "test-macos", "conclusion": "failure"}],
                },
            },
        )
        proc = command("ciprofile.py", "durations", "2026-11-01T00:00:00Z", "2026-11-02T00:00:00Z", env=env)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn(
            "completed non-release PR runs created 2026-11-01T00:00:00Z..2026-11-02T00:00:00Z: 1", proc.stdout
        )
        self.assertIn("test-linux    n=  1 (too few for quartiles)", proc.stdout)
        self.assertIn("test-macos    n=  0 (too few for quartiles)", proc.stdout)

    def test_history_takes_the_first_or_last_run_and_names_a_pr_with_none(self):
        root = scratch(self)
        cache = fixture_cache(root)
        later = {**RUN_RECORD, "id": 90, "created_at": "2026-11-12T09:00:00Z", "pull_requests": []}
        shutil.copytree(cache / f"runs/{RUN}", cache / "runs/90")
        (cache / "runs/90/run.json").write_text(json.dumps(later))
        listing = cache / "lists/event_pull_request_created_>_2026-11-01.1.json"
        listing.parent.mkdir(parents=True)
        listing.write_text(json.dumps({"workflow_runs": [later, RUN_RECORD]}))
        prs = root / "prs.json"
        prs.write_text(
            json.dumps([{"pr": 9, "head": "a" * 40, "branch": "topic"}, {"pr": 10, "head": "d" * 40, "branch": "x"}])
        )
        env = {"CIPROFILE_CACHE": str(cache), "CIPROFILE_OFFLINE": "1"}
        for which, run in (("first", RUN), ("last", 90)):
            out = root / f"{which}.json"
            proc = command("ciprofile.py", "history", str(prs), "--out", str(out), "--which", which, env=env)
            self.assertEqual(proc.returncode, 4)
            self.assertIn("no run since 2026-11-01 for #10", proc.stderr)
            entry = json.loads(out.read_text())["9"]
            self.assertEqual((entry["run"], entry["runs_of_pr"]), (run, 2))


class ClientTests(unittest.TestCase):
    """The GitHub client against a stand-in `gh`: floors, paging and artifacts."""

    def client(self, root: Path) -> ghcache.GitHub:
        return ghcache.GitHub(cache=root / "cache", offline=False)

    def calls(self, root: Path) -> list[str]:
        return (root / "calls.log").read_text().splitlines()

    def test_the_graphql_floor_refuses_and_the_allowance_is_read_again_every_batch(self):
        root = scratch(self)
        env = fake_gh(root, remaining=4000, graphql=100, responses={})
        with unittest.mock.patch.dict("os.environ", env):
            with self.assertRaisesRegex(ghcache.RateFloor, "only 100 GraphQL"):
                asyncio.run(self.client(root).run(models.RunId(1)))
        root = scratch(self)
        responses = {f"actions/runs/{i}": {**RUN_RECORD, "id": i} for i in range(1, 28)}
        env = fake_gh(root, remaining=4000, responses=responses)
        with unittest.mock.patch.dict("os.environ", env):
            gh = self.client(root)

            async def read_all() -> None:
                for i in range(1, 28):
                    await gh.run(models.RunId(i))

            asyncio.run(read_all())
        self.assertEqual(self.calls(root).count("api rate_limit"), 2)  # before read 1 and before read 26

    def test_jobs_are_paged_to_their_end(self):
        root = scratch(self)
        page = [{**JOB, "id": i} for i in range(1, 101)]
        env = fake_gh(
            root,
            remaining=4000,
            responses={
                f"actions/runs/{RUN}/jobs?per_page=100&page=1": {"total_count": 101, "jobs": page},
                f"actions/runs/{RUN}/jobs?per_page=100&page=2": {"total_count": 101, "jobs": [{**JOB, "id": 101}]},
            },
        )
        with unittest.mock.patch.dict("os.environ", env):
            jobs = asyncio.run(self.client(root).jobs(models.RunId(RUN)))
        self.assertEqual(len(jobs), 101)

    def artifact_gh(self, root: Path, entries: dict[str, bytes], expired: bool = False) -> dict:
        data = io.BytesIO()
        with zipfile.ZipFile(data, "w") as archive:
            for name, body in entries.items():
                archive.writestr(name, body)
        (root / "artifact.zip").write_bytes(data.getvalue())
        env = fake_gh(
            root,
            remaining=4000,
            responses={
                f"actions/runs/{RUN}/artifacts?per_page=100&page=1": {
                    "total_count": 1,
                    "artifacts": [{"id": 3, "name": "spike-profile-test-macos", "expired": expired}],
                },
            },
        )
        script = Path(env["PATH"].split(":")[0]) / "gh"
        script.write_text(
            script.read_text().replace(
                "responses = json.load(",
                textwrap.dedent(f"""\
            if args[1].endswith("actions/artifacts/3/zip"):
                sys.stdout.buffer.write(open({str(root / "artifact.zip")!r}, "rb").read())
                sys.exit(0)
            responses = json.load("""),
            )
        )
        return env

    def test_an_artifact_is_downloaded_once_and_only_its_records_kept(self):
        root = scratch(self)
        env = self.artifact_gh(root, {"tests.jsonl": b"{}\n", "big.tar": b"x" * 10})
        with unittest.mock.patch.dict("os.environ", env):
            gh = self.client(root)
            for _ in range(2):
                self.assertEqual(
                    asyncio.run(gh.artifact_member(models.RunId(RUN), "spike-profile-test-macos", "tests.jsonl")),
                    b"{}\n",
                )
            self.assertIsNone(asyncio.run(gh.artifact_member(models.RunId(RUN), "spike-profile-test-macos", "big.tar")))
        self.assertEqual(sum("artifacts/3/zip" in c for c in self.calls(root)), 1)

    def test_an_entry_outside_its_directory_and_an_expired_artifact_are_refused(self):
        root = scratch(self)
        env = self.artifact_gh(root, {"../escape.json": b"{}"})
        with unittest.mock.patch.dict("os.environ", env):
            with self.assertRaisesRegex(models.Invalid, "outside its directory"):
                asyncio.run(self.client(root).artifact_member(models.RunId(RUN), "spike-profile-test-macos", "x.json"))
        self.assertFalse((root / "cache/runs" / f"{RUN}" / "artifacts/escape.json").exists())
        root = scratch(self)
        env = self.artifact_gh(root, {"tests.jsonl": b"{}"}, expired=True)
        with unittest.mock.patch.dict("os.environ", env):
            self.assertIsNone(
                asyncio.run(
                    self.client(root).artifact_member(models.RunId(RUN), "spike-profile-test-macos", "tests.jsonl")
                )
            )


class AttributionPathTests(unittest.TestCase):
    def profile(self, log_lines: list[str], tasks: list[dict]) -> prof.JobProfile:
        return prof.profile_job(
            models.Job.parse(JOB),
            "\n".join([*LOG.splitlines()[:6], *log_lines]),
            models.NxExecution.report({"tasks": tasks}),
        )

    def task(self, name: str, start: str, end: str, result: str = "executed") -> dict:
        return {
            "task": name,
            "result": result,
            "started_at": f"2026-11-12T{start}.000Z",
            "finished_at": f"2026-11-12T{end}.000Z",
        }

    def test_each_kind_of_task_second_lands_in_its_own_bucket(self):
        p = self.profile(
            [
                "2026-11-12T04:01:10.0000000Z [scripts:distribution-test] scripts:    Compiling onetaskgraph v0",
                "2026-11-12T04:02:10.0000000Z [scripts:distribution-test] scripts:     Finished `release` profile",
                "2026-11-12T04:01:40.0000000Z [coverage] "
                '{"type":"test","event":"started","name":"c-crate::e2e$journey"}',
                "2026-11-12T04:02:00.0000000Z [coverage] "
                '{"type":"test","event":"ok","name":"c-crate::e2e$journey","exec_time":19.5}',
                "2026-11-12T04:03:00.0000000Z [test] > nx run d-crate:test",
                "2026-11-12T04:03:30.0000000Z [test] d-crate: running 1 test",
            ],
            [
                self.task("scripts:distribution-test", "04:01:06", "04:03:06"),
                self.task("c-crate:coverage", "04:01:06", "04:02:06"),
                self.task("c-crate:budgets", "04:02:06", "04:02:36"),
                self.task("workspace:test", "04:02:36", "04:02:56"),
                self.task("c-crate:bootstrap", "04:01:06", "04:01:07"),
            ],
        )
        self.assertEqual(
            {t.id for t in p.tasks},
            {"scripts:distribution-test", "c-crate:coverage", "c-crate:budgets", "workspace:test", "d-crate:test"},
        )  # d from the log, bootstrap left out
        # 04:01:10–04:02:10 shared with one other task throughout: half of 60 s.
        self.assertAlmostEqual(p.buckets[prof.Bucket.COMPILE_RELEASE], 30, delta=0.01)
        self.assertGreater(p.buckets[prof.Bucket.COMPILE_INSTRUMENTED], 0)
        self.assertGreater(p.buckets[prof.Bucket.TEST_INSTRUMENTED], 0)
        self.assertGreater(p.buckets[prof.Bucket.BUDGETS], 0)
        self.assertGreater(p.buckets[prof.Bucket.SCRIPTS], 0)
        self.assertEqual(p.nextest, [models.TestTime("instrumented:c-crate::e2e", "journey", 19.5)])
        self.assertAlmostEqual(sum(p.buckets.values()), p.wall, places=6)

    def test_a_setup_actions_own_restore_is_cache_restore(self):
        job = {
            **JOB,
            "steps": [
                *STEPS[:1],
                {
                    "name": "Run astral-sh/setup-uv@v7",
                    "started_at": "2026-11-12T04:00:10Z",
                    "completed_at": "2026-11-12T04:01:00Z",
                    "conclusion": "success",
                },
            ],
        }
        log = "\n".join(
            [
                "2026-11-12T04:00:10.0000000Z ##[group]Run astral-sh/setup-uv@v7",
                "2026-11-12T04:00:20.0000000Z Trying to restore uv cache from GitHub Actions cache with key: x",
                "2026-11-12T04:00:32.0000000Z uv cache restored from GitHub Actions cache with key: x",
                "2026-11-12T04:01:00.0000000Z ##[group]Run something else",
            ]
        )
        p = prof.profile_job(models.Job.parse(job), log, models.NxExecution.report(EXECUTIONS))
        self.assertAlmostEqual(p.buckets[prof.Bucket.CACHE_RESTORE], 12, places=3)


if __name__ == "__main__":
    unittest.main()


FIXTURES = HERE / "data/github"


class GitHubResponseTests(unittest.TestCase):
    """models.py's GitHub records against GitHub's own answers to this harness's reads (data/github/README.md)."""

    def test_recorded_jobs_runs_artifacts_and_allowances_parse(self):
        jobs = [models.Job.parse(j) for j in json.loads((FIXTURES / "run-43152201170-jobs.json").read_text())["jobs"]]
        self.assertEqual({j.name for j in jobs} >= {"test-linux", "test-macos", "test-windows"}, True)
        self.assertTrue(all(j.finished for j in jobs if j.name.startswith("test-")))
        runs = [
            models.Run.parse(r) for r in json.loads((FIXTURES / "runs-listing-page.json").read_text())["workflow_runs"]
        ]
        self.assertEqual(len(runs), 3)
        artifacts = json.loads((FIXTURES / "run-43369924587-artifacts.json").read_text())["artifacts"]
        self.assertIn("spike-profile-test-macos", {models.Artifact.parse(a).name for a in artifacts})
        limit = models.RateLimit.parse(json.loads((FIXTURES / "rate-limit.json").read_text()))
        self.assertGreaterEqual(limit.rest, 0)

    def test_a_skipped_job_github_stamps_as_ending_before_it_starts_is_read_as_never_run(self):
        jobs = [models.Job.parse(j) for j in json.loads((FIXTURES / "run-43392807274-jobs.json").read_text())["jobs"]]
        skipped = {j.name: j for j in jobs if j.conclusion is models.Conclusion.SKIPPED}
        self.assertEqual(set(skipped), {"lint", "budgets"})
        self.assertTrue(all(not j.finished for j in skipped.values()))
        self.assertTrue(all(j.finished for j in jobs if j.name.startswith("test-")))

    def test_every_conclusion_github_answered_is_one_the_model_knows(self):
        seen = set()
        for job in json.loads((FIXTURES / "run-43152201170-jobs.json").read_text())["jobs"]:
            seen |= {job["conclusion"], *(s["conclusion"] for s in job["steps"])}
        for run in json.loads((FIXTURES / "runs-listing-page.json").read_text())["workflow_runs"]:
            seen.add(run["conclusion"])
        self.assertLessEqual(seen - {None}, {c.value for c in models.Conclusion})


class MoreCommandPathTests(unittest.TestCase):
    def test_failed_tests_and_nextest_lines_reach_the_report_and_top_truncates(self):
        root = scratch(self)
        cache = fixture_cache(root)
        run = cache / f"runs/{RUN}"
        (run / "logs/5.txt").write_text(
            LOG
            + "\n"
            + "\n".join(
                f"2026-11-12T04:05:0{i}.0000000Z [coverage] "
                + json.dumps(
                    {"type": "test", "event": "ok", "name": f"x::e2e$t{i}", "exec_time": i + 0.5}, separators=(",", ":")
                )
                for i in range(3)
            )
        )
        (run / "artifacts/spike-profile-test-macos/tests.jsonl").write_text(
            "\n".join(
                json.dumps(e)
                for e in [
                    {
                        "type": "test",
                        "event": "failed",
                        "binary": "e2e-0123456789abcdef",
                        "name": "broke",
                        "exec_time": 9.0,
                    },
                    {
                        "type": "suite",
                        "event": "failed",
                        "binary": "e2e-0123456789abcdef",
                        "passed": 0,
                        "failed": 0,
                        "ignored": 0,
                        "filtered_out": 0,
                        "exec_time": 9.0,
                    },
                ]
            )
        )
        env = {"CIPROFILE_CACHE": str(cache), "CIPROFILE_OFFLINE": "1"}
        proc = command("ciprofile.py", "profile", str(RUN), "--jobs", "test-macos", "--top", "1", env=env)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("9.0s  e2e-0123456789abcdef::broke", proc.stdout)
        self.assertIn("2.5s  instrumented:x::e2e::t2", proc.stdout)
        binaries = proc.stdout.split("test binaries (2, seconds):\n")[1].split("  slowest")[0].strip().splitlines()
        self.assertEqual(len(binaries), 1)
        tasks = proc.stdout.split("Nx tasks by share of wall clock (own seconds, result):\n")[1].split("  latest")[0]
        self.assertEqual(len(tasks.strip().splitlines()), 1)

    def test_without_its_artifacts_a_job_is_profiled_from_its_log_alone(self):
        root = scratch(self)
        cache = fixture_cache(root)
        shutil.rmtree(cache / f"runs/{RUN}/artifacts")
        (cache / f"runs/{RUN}/artifacts.json").write_text(json.dumps({"total_count": 0, "artifacts": []}))
        proc = command(
            "ciprofile.py",
            "profile",
            str(RUN),
            "--jobs",
            "test-macos",
            env={"CIPROFILE_CACHE": str(cache), "CIPROFILE_OFFLINE": "1"},
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("Nx tasks: 1 run, 1 replayed [log headers]", proc.stdout)
        self.assertNotIn("sampler:", proc.stdout)

    def test_a_run_already_listed_is_not_read_again_and_a_failed_read_exits_4(self):
        root = scratch(self)
        env = fake_gh(root, remaining=4000, responses={})
        cache = Path(env["CIPROFILE_CACHE"])
        (cache / "lists").mkdir(parents=True)
        (cache / "lists/page.1.json").write_text(json.dumps({"workflow_runs": [RUN_RECORD]}))
        with unittest.mock.patch.dict("os.environ", env):
            run = asyncio.run(ghcache.GitHub(cache=cache, offline=False).run(models.RunId(RUN)))
        self.assertEqual(run.head_branch, "topic")
        self.assertFalse((root / "calls.log").exists())
        proc = command("ciprofile.py", "profile", "999", env=env)
        self.assertEqual(proc.returncode, 4)
        self.assertIn(
            "ReadFailed: gh api --allow-escape-sequences repos/nickderobertis/onetaskgraph/actions/runs/999: HTTP 404",
            proc.stderr,
        )

    def test_the_replay_matches_each_pr_against_the_keys_in_force_when_it_ran(self):
        root = scratch(self)
        for side, files in (("after", ["crates/c/src/new.rs"]), ("before", ["crates/c/src/old.rs"])):
            (root / side).mkdir()
            (root / f"{side}/c__test.json").write_text(json.dumps({"files": files}))
        record = {
            "run": 1,
            "head_sha": "e" * 40,
            "created_at": "2026-11-01T00:00:00Z",
            "conclusion": "success",
            "runs_of_pr": 1,
            "jobs": {
                "test-linux": {
                    "job_id": 1,
                    "conclusion": "success",
                    "wall_s": 60,
                    "source": "log",
                    "tasks": {"c:test": {"result": "executed", "seconds": 60, "share_s": 60}},
                }
            },
        }
        (root / "h.json").write_text(json.dumps({"3900": record, "3990": {**record, "run": 2}}))
        (root / "p.json").write_text(
            json.dumps([{"pr": 3900, "files": ["crates/c/src/old.rs"]}, {"pr": 3990, "files": ["crates/c/src/new.rs"]}])
        )
        proc = command(
            "replay.py",
            str(root / "h.json"),
            str(root / "p.json"),
            str(root / "after"),
            str(root / "before"),
            "--json",
            str(root / "r.json"),
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        rows = json.loads((root / "r.json").read_text())["rows"]
        self.assertEqual(rows["3900"]["test-linux"]["c:test"]["matched"], ["crates/c/src/old.rs"])
        self.assertEqual(rows["3990"]["test-linux"]["c:test"]["matched"], ["crates/c/src/new.rs"])


class RunnerPassthroughTests(unittest.TestCase):
    def binary(self, root: Path) -> Path:
        path = root / "probe-0123456789abcdef"
        path.write_text("#!/usr/bin/env python3\nimport sys\nprint('args=' + ' '.join(sys.argv[1:]))\nsys.exit(7)\n")
        path.chmod(0o755)
        return path

    def test_a_list_probe_a_nextest_child_and_an_unset_record_run_untouched(self):
        root = scratch(self)
        record = root / "tests.jsonl"
        for args, env in (
            (["--list"], {"SPIKE_TEST_RECORD": str(record)}),
            (["-q"], {"SPIKE_TEST_RECORD": str(record), "NEXTEST_RUN_ID": "1"}),
            (["-q"], {}),
        ):
            proc = command("testrunner.py", str(self.binary(root)), *args, env=env)
            self.assertEqual(proc.returncode, 7)
            self.assertEqual(proc.stdout.strip(), "args=" + " ".join(args))
        spawns = [json.loads(x) for x in (root / "tests.jsonl.spawns").read_text().splitlines()]
        self.assertEqual([(s["mode"], s["code"]) for s in spawns], [("passthrough", 7), ("passthrough", 7)])
        self.assertFalse(record.exists())


class BoundaryFailureTests(unittest.TestCase):
    def test_a_window_that_does_not_move_forward_is_a_usage_error(self):
        for since, until in (("2026-11-02T00:00:00Z", "2026-11-01T00:00:00Z"), ("2026-11-01T00:00:00Z",) * 2):
            proc = command("ciprofile.py", "durations", since, until, env={"CIPROFILE_OFFLINE": "1"})
            self.assertEqual(proc.returncode, 2)
            self.assertIn("the window's end must follow its start", proc.stderr)

    def test_a_pr_listed_twice_or_numbered_zero_exits_4(self):
        root = scratch(self)
        env = {"CIPROFILE_CACHE": str(fixture_cache(root)), "CIPROFILE_OFFLINE": "1"}
        for prs, why in (
            ([{"pr": 9, "head": "a", "branch": "x"}] * 2, "listed twice"),
            ([{"pr": 0, "head": "a", "branch": "x"}], "positive"),
        ):
            (root / "prs.json").write_text(json.dumps(prs))
            proc = command("ciprofile.py", "history", str(root / "prs.json"), "--out", str(root / "h.json"), env=env)
            self.assertEqual(proc.returncode, 4)
            self.assertIn(why, proc.stderr)

    def test_a_malformed_pr_list_and_an_unwritable_output_exit_4_naming_why(self):
        root = scratch(self)
        cache = fixture_cache(root)
        listing = cache / "lists/event_pull_request_created_>_2026-11-01.1.json"
        listing.parent.mkdir(parents=True)
        listing.write_text(json.dumps({"workflow_runs": [RUN_RECORD]}))
        env = {"CIPROFILE_CACHE": str(cache), "CIPROFILE_OFFLINE": "1"}
        (root / "bad.json").write_text("{not json")
        proc = command("ciprofile.py", "history", str(root / "bad.json"), "--out", str(root / "h.json"), env=env)
        self.assertEqual(proc.returncode, 4)
        self.assertIn("JSONDecodeError", proc.stderr)
        (root / "prs.json").write_text(json.dumps([{"pr": 9, "head": "a" * 40, "branch": "topic"}]))
        proc = command(
            "ciprofile.py", "history", str(root / "prs.json"), "--out", str(root / "no/such/dir/h.json"), env=env
        )
        self.assertEqual(proc.returncode, 4)
        self.assertIn("FileNotFoundError", proc.stderr)


class ReplayOptionJourneyTests(unittest.TestCase):
    def test_a_formatter_config_edit_would_have_replayed_the_rust_tests_it_reran(self):
        root = scratch(self)
        (root / "after").mkdir()
        (root / "after/onetaskgraph-core__test.json").write_text(
            json.dumps({"files": ["biome.json", "crates/x/src/a.rs"]})
        )
        record = {
            "run": 1,
            "head_sha": "f" * 40,
            "created_at": "2026-11-12T00:00:00Z",
            "conclusion": "success",
            "runs_of_pr": 1,
            "jobs": {
                "test-windows": {
                    "job_id": 1,
                    "conclusion": "success",
                    "wall_s": 300,
                    "source": "executions",
                    "tasks": {"onetaskgraph-core:test": {"result": "executed", "seconds": 200, "share_s": 90}},
                }
            },
        }
        (root / "h.json").write_text(json.dumps({"3990": record}))
        (root / "p.json").write_text(json.dumps([{"pr": 3990, "files": ["biome.json"]}]))
        proc = command(
            "replay.py", str(root / "h.json"), str(root / "p.json"), str(root / "after"), str(root / "after")
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn(
            "non-semantic-globals-out-of-rust-keys: test-linux 0.0 (0.00/PR)  test-macos 0.0 (0.00/PR)  "
            "test-windows 1.5 (1.50/PR)",
            proc.stdout,
        )


@dataclasses.dataclass(frozen=True)
class FakeTimes:
    user: float
    system: float = 0.0


@dataclasses.dataclass
class FakeProcess:
    """One entry of a stand-in process table: what psutil reports of a process."""

    pid: int
    ppid: int
    name: str
    created: float
    cpu: float
    command: tuple[str, ...] = ()
    unreadable_command: bool = False
    unreadable_cpu: bool = False

    @property
    def info(self) -> dict[str, Any]:
        return {"pid": self.pid, "ppid": self.ppid, "name": self.name, "create_time": self.created}

    def cmdline(self) -> list[str]:
        if self.unreadable_command:
            raise OSError("access denied")
        return list(self.command)

    def cpu_times(self) -> FakeTimes:
        if self.unreadable_cpu:
            raise OSError("access denied")
        return FakeTimes(self.cpu)


class FakeProcessTable:
    """The OS boundary the sampler reads, as psutil presents it."""

    Error: type[Exception] = OSError

    def __init__(self) -> None:
        self.processes: list[FakeProcess] = []

    def process_iter(self, attrs: list[str]) -> list[FakeProcess]:
        del attrs  # psutil reads only the named fields; the stand-in holds every one
        return list(self.processes)


class SamplerAccountingTests(unittest.TestCase):
    def test_a_working_descendant_hides_its_ancestors_and_the_sidecar_is_nobodys_work(self):

        table = FakeProcessTable()
        table.processes = [
            FakeProcess(10, 1, "bash", 0.0, 0.0),
            FakeProcess(11, 10, "cargo", 0.0, 0.0),
            FakeProcess(12, 11, "rustc", 0.0, 0.0),
            FakeProcess(20, 1, "node", 0.0, 0.0, command=("node", "/a/op-nx/start-cache-server/main.js")),
            FakeProcess(21, 20, "gtar", 0.0, 0.0),
            FakeProcess(40, 1, "bash", 0.0, 0.0),
            FakeProcess(41, 40, "node", 0.0, 0.0, command=("node", "/a/op-nx/start-cache-server/main.js")),
        ]
        taken = sampler.sample(table, {}, me=999, previous=100.0, tick=1.0)
        # rustc hides cargo and bash; gtar hides the sidecar that runs it; a bare sidecar hides nothing above it.
        self.assertEqual(
            sampler.leaves(taken.alive),
            {
                models.Category.COMPILE: 1,
                models.Category.ARCHIVE: 1,
                models.Category.SHELL: 1,
                models.Category.NX_CACHE_SIDECAR: 1,
            },
        )

    def test_cpu_is_counted_from_when_the_sampler_first_saw_a_process_and_a_reused_pid_is_a_new_process(self):

        table, seen = FakeProcessTable(), {}
        old = FakeProcess(30, 1, "python3", created=1.0, cpu=50.0)  # running long before the sampler started
        fresh = FakeProcess(31, 1, "git", created=100.5, cpu=0.2)  # started inside this tick
        table.processes = [old, fresh]
        first = sampler.sample(table, seen, me=999, previous=100.0, tick=1.0)
        self.assertEqual(first.cpu.get(models.Category.PYTHON, 0.0), 0.0)
        self.assertAlmostEqual(first.cpu[models.Category.GIT], 0.2)
        old.cpu = 51.5
        table.processes = [old, FakeProcess(31, 1, "rustc", created=101.6, cpu=0.3)]  # pid 31 reused by a new process
        second = sampler.sample(table, seen, me=999, previous=101.0, tick=1.0)
        self.assertAlmostEqual(second.cpu[models.Category.PYTHON], 1.5)
        self.assertEqual(second.alive[31].category, models.Category.COMPILE)
        self.assertAlmostEqual(second.cpu[models.Category.COMPILE], 0.3)
        table.processes = []
        gone = sampler.exits(seen, {})
        self.assertEqual(
            {(e.pid, e.category) for e in gone}, {(30, models.Category.PYTHON), (31, models.Category.COMPILE)}
        )


class MoreProducerContractTests(unittest.TestCase):
    def test_each_jobs_runner_target_is_the_triple_of_the_image_it_runs_on(self):

        workflow = (REPO / ".github/workflows/ci.yml").read_text()
        images = dict(re.findall(r"\n  (test-[a-z]+):\n(?:    [^\n]*\n)*?    runs-on: ([\w.-]+)", workflow))
        expected = {
            "ubuntu-latest": "X86_64_UNKNOWN_LINUX_GNU",
            "macos-latest": "AARCH64_APPLE_DARWIN",
            "windows-latest": "X86_64_PC_WINDOWS_MSVC",
        }
        self.assertEqual(set(images), set(measurement_only.TRIPLES))
        for job, image in images.items():
            self.assertEqual(measurement_only.TRIPLES[job], expected[image], f"{job} runs on {image}")

    def test_the_stored_entries_line_is_the_one_the_cache_report_prints(self):
        source = (REPO / "scripts/nx-remote-cache.mjs").read_text()
        template = re.search(
            r"`(Nx cache: stored \$\{.*?\} entries, \$\{.*?\} bytes; replayed \$\{.*?\} entries)`", source
        )
        assert template is not None, "nx-remote-cache.mjs no longer prints its stored-entries line"
        rendered = re.sub(r"\$\{(?:[^{}]|\{[^{}]*\})*\}", "7", template.group(1))
        match = prof.STORED.search(rendered)
        assert match is not None, f"{rendered!r} is not read by jobprofile.STORED"
        self.assertEqual((match.group("n"), match.group("bytes"), match.group("r")), ("7", "7", "7"))


class CacheRecoveryTests(unittest.TestCase):
    def test_a_missing_member_is_answered_from_the_cache_and_an_interrupted_extraction_is_redone(self):
        root = scratch(self)
        env = ClientTests().artifact_gh(root, {"tests.jsonl": b"{}\n"})
        with unittest.mock.patch.dict("os.environ", env):
            gh = ghcache.GitHub(cache=root / "cache", offline=False)
            for _ in range(2):
                self.assertIsNone(
                    asyncio.run(gh.artifact_member(models.RunId(RUN), "spike-profile-test-macos", "x.json"))
                )
            downloads = sum("artifacts/3/zip" in c for c in (root / "calls.log").read_text().splitlines())
            self.assertEqual(downloads, 1)
            directory = root / f"cache/runs/{RUN}/artifacts/spike-profile-test-macos"
            (directory / ghcache.COMPLETE).unlink()
            (directory / "tests.jsonl").unlink()
            self.assertEqual(
                asyncio.run(gh.artifact_member(models.RunId(RUN), "spike-profile-test-macos", "tests.jsonl")), b"{}\n"
            )
        self.assertEqual(sum("artifacts/3/zip" in c for c in (root / "calls.log").read_text().splitlines()), 2)

    def test_a_truncated_legacy_listing_is_paged_again(self):
        root = scratch(self)
        env = fake_gh(
            root,
            remaining=4000,
            responses={
                f"actions/runs/{RUN}/jobs?per_page=100&page=1": {"total_count": 2, "jobs": [JOB, {**JOB, "id": 6}]}
            },
        )
        legacy = root / f"cache/runs/{RUN}/jobs.json"
        legacy.parent.mkdir(parents=True)
        legacy.write_text(json.dumps({"total_count": 2, "jobs": [JOB]}))
        with unittest.mock.patch.dict("os.environ", env):
            jobs = asyncio.run(ghcache.GitHub(cache=root / "cache", offline=False).jobs(models.RunId(RUN)))
        self.assertEqual([j.id for j in jobs], [5, 6])


class LogFallbackTests(unittest.TestCase):
    def test_a_local_replay_in_the_log_stays_a_local_hit(self):
        log = "\n".join(
            [*LOG.splitlines()[:6], "2026-11-12T04:01:06.0000000Z [test,budgets] > nx run e-crate:test  [local cache]"]
        )
        p = prof.profile_job(models.Job.parse(JOB), log, None)
        self.assertEqual({t.id: t.result for t in p.tasks}, {"e-crate:test": models.TaskResult.LOCAL_HIT})


class ShimInstallTests(unittest.TestCase):
    @unittest.skipUnless(shutil.which("rustc"), "compiles the shim with rustc")
    def test_runners_installing_the_shim_at_once_all_run_their_binary(self):
        root = scratch(self)
        binary = root / "probe-0123456789abcdef"
        binary.write_text(
            "#!/usr/bin/env python3\nimport json\n"
            "print(json.dumps({'type': 'suite', 'event': 'started', 'test_count': 0}))\n"
            "print(json.dumps({'type': 'suite', 'event': 'ok', 'passed': 0, 'failed': 0,"
            " 'ignored': 0, 'filtered_out': 0, 'exec_time': 0.0}))\n"
        )
        binary.chmod(0o755)
        record = root / "spike" / "tests.jsonl"
        record.parent.mkdir()
        env = {**__import__("os").environ, "SPIKE_TEST_RECORD": str(record), "CARGO": sys.executable}
        runners = [
            subprocess.Popen(
                [sys.executable, str(HERE / "testrunner.py"), str(binary)], env=env, stdout=subprocess.DEVNULL
            )
            for _ in range(4)
        ]
        self.assertEqual([r.wait(timeout=300) for r in runners], [0, 0, 0, 0])
        self.assertTrue((record.parent / "cargo-shim/cargo").exists())
        self.assertEqual(len([p for p in (record.parent / "cargo-shim").iterdir() if p.is_dir()]), 0)


class ReplayRefusalTests(unittest.TestCase):
    def history_with(self, root: Path, pr: str, task: str) -> None:
        record = {
            "run": 1,
            "head_sha": "e" * 40,
            "created_at": "2026-11-12T00:00:00Z",
            "conclusion": "success",
            "runs_of_pr": 1,
            "jobs": {
                "test-linux": {
                    "job_id": 1,
                    "conclusion": "success",
                    "wall_s": 60,
                    "source": "log",
                    "tasks": {task: {"result": "executed", "seconds": 60, "share_s": 60}},
                }
            },
        }
        (root / "h.json").write_text(json.dumps({pr: record}))
        (root / "after").mkdir()
        (root / "after/c__test.json").write_text(json.dumps({"files": ["crates/c/src/a.rs"]}))

    def run_replay(self, root: Path, *extra: str):
        return command(
            "replay.py", str(root / "h.json"), str(root / "p.json"), str(root / "after"), str(root / "after"), *extra
        )

    def test_a_pr_the_history_names_but_the_list_does_not_exits_2_naming_it(self):
        root = scratch(self)
        self.history_with(root, "3990", "c:test")
        (root / "p.json").write_text(json.dumps([{"pr": 3991, "files": []}]))
        proc = self.run_replay(root)
        self.assertEqual(proc.returncode, 2)
        self.assertIn("#3990 is in the history but not in the PR list", proc.stderr)

    def test_a_target_with_no_resolved_inputs_is_reported_unresolved(self):
        root = scratch(self)
        self.history_with(root, "3990", "d:test")
        (root / "p.json").write_text(json.dumps([{"pr": 3990, "files": ["crates/d/src/a.rs"]}]))
        proc = self.run_replay(root, "--json", str(root / "r.json"))
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("test-linux: (not resolved) 1", proc.stdout)
        reason = json.loads((root / "r.json").read_text())["rows"]["3990"]["test-linux"]["d:test"]
        self.assertEqual((reason["matched"], reason["why"]), (None, "inputs not resolved for this target"))

    def test_a_missing_inputs_directory_and_a_malformed_history_time_are_refused(self):
        root = scratch(self)
        self.history_with(root, "3990", "c:test")
        (root / "p.json").write_text(json.dumps([{"pr": 3990, "files": []}]))
        proc = command(
            "replay.py", str(root / "h.json"), str(root / "p.json"), str(root / "nowhere"), str(root / "after")
        )
        self.assertEqual(proc.returncode, 2)
        self.assertIn("expected a directory", proc.stderr)
        history = json.loads((root / "h.json").read_text())
        history["3990"]["created_at"] = "yesterday"
        with self.assertRaisesRegex(models.Invalid, "not an ISO-8601 time"):
            models.history(history)


class TickDivisionTests(unittest.TestCase):
    def test_an_idle_subprocess_no_leaves_and_busy_orchestration_each_have_their_own_name(self):
        records = [
            models.Tick(1.0, 1.0, {models.Category.GIT: 1}, {}),
            models.Tick(2.0, 1.0, {}, {}),
            models.Tick(3.0, 1.0, {models.Category.NX: 1}, {models.Category.GIT: 0.5}),
        ]
        division = prof.by_process(records, prof.Interval(0, 10))
        self.assertEqual(
            division.seconds,
            {"wait:subprocess(git)": 1.0, "wait:subprocess(none)": 1.0, "orchestration(nx,shell)": 1.0},
        )


class ListingAndRecorderEdgeTests(unittest.TestCase):
    def test_a_run_listing_is_paged_up_to_its_limit(self):
        root = scratch(self)
        base = "actions/workflows/ci.yml/runs?event=pull_request&per_page=100&page="
        full = [{**RUN_RECORD, "id": 1000 + i} for i in range(100)]
        env = fake_gh(
            root,
            remaining=4000,
            responses={
                f"{base}1": {"workflow_runs": full},
                f"{base}2": {"workflow_runs": full},
                f"{base}3": {"workflow_runs": [RUN_RECORD]},
            },
        )
        with unittest.mock.patch.dict("os.environ", env):
            gh = ghcache.GitHub(cache=root / "cache", offline=False)
            self.assertEqual(len(asyncio.run(gh.runs_list("event=pull_request", pages=2))), 200)
            self.assertEqual(len(asyncio.run(gh.runs_list("event=pull_request", pages=5))), 201)
        pages = [c for c in (root / "calls.log").read_text().splitlines() if "page=" in c]
        self.assertEqual(len(pages), 3)  # the second listing reread no cached page

    def test_a_process_whose_cpu_cannot_be_read_keeps_its_last_reading(self):
        table, seen = FakeProcessTable(), {}
        flaky = FakeProcess(50, 1, "git", created=100.5, cpu=0.4)
        table.processes = [flaky]
        sampler.sample(table, seen, me=999, previous=100.0, tick=1.0)

        flaky.unreadable_cpu = True
        second = sampler.sample(table, seen, me=999, previous=101.0, tick=1.0)
        self.assertEqual(second.cpu.get(models.Category.GIT, 0.0), 0.0)
        self.assertEqual(seen[50].last_cpu, 0.4)
        self.assertIn(50, second.alive)

    def test_a_tick_far_longer_than_the_interval_names_its_excess_unsampled(self):
        division = prof.by_process(
            [models.Tick(9.0, 8.0, {models.Category.COMPILE: 1}, {models.Category.COMPILE: 8.0})], prof.Interval(0, 10)
        )
        self.assertEqual(division.seconds, {"compile": 5.0, "unsampled(recorder stalled)": 3.0})

    def test_duplicate_and_non_canonical_pull_requests_are_refused(self):
        with self.assertRaisesRegex(models.Invalid, "listed twice"):
            replay.parse_prs([{"pr": 1, "files": []}, {"pr": 1, "files": []}])
        with self.assertRaisesRegex(models.Invalid, "canonically"):
            models.history({"01": {}})
        with self.assertRaisesRegex(models.Invalid, "positive"):
            replay.parse_prs([{"pr": 0, "files": []}])
        with self.assertRaisesRegex(models.Invalid, "normalized"):
            replay.parse_prs([{"pr": 1, "files": ["."]}])


LINES = (HERE / "data/logs/run-43152201170-lines.txt").read_text(encoding="utf-8").splitlines()


class RealLogLineTests(unittest.TestCase):
    """Each log format jobprofile.py reads, held to lines real job logs printed (data/logs/README.md)."""

    def texts(self) -> list[str]:
        return [line.text for raw in LINES for line in prof.parse_log(raw)]

    def test_every_real_line_has_a_github_timestamp(self):
        self.assertEqual(len(self.texts()), len(LINES))

    def test_nx_headers_executed_and_replayed_are_read(self):
        headers = [m for t in self.texts() if (m := prof.HEADER.match(t))]
        self.assertTrue(any(m.group("remote") for m in headers))
        self.assertTrue(any(not m.group("remote") for m in headers))
        self.assertTrue(all(models.TASK_ID.fullmatch(m.group("task")) for m in headers))

    def test_test_starts_nextest_events_and_cargo_build_lines_are_read(self):
        texts = self.texts()
        self.assertTrue(any(prof.TEST_START.match(t) for t in texts))
        nextest = [m for t in texts if (m := prof.NEXTEST_DONE.match(t)) and m.group("secs")]
        self.assertTrue(nextest and all("::" in m.group("name") and "$" in m.group("name") for m in nextest))
        builds = [
            cm for t in texts if (m := prof.PREFIXED.match(t)) and (cm := prof.CARGO_BUILD.match(m.group("text")))
        ]
        self.assertEqual({cm.group("word") for cm in builds}, {"Compiling", "Finished"})
        self.assertTrue(any(cm.group("profile") for cm in builds if cm.group("word") == "Finished"))
        self.assertTrue(
            any((m := prof.PREFIXED.match(t)) and m.group("text").startswith("Blocking waiting") for t in texts)
        )

    def test_the_cache_size_and_stored_entries_are_read(self):
        texts = self.texts()
        self.assertTrue(any(prof.CACHE_SIZE.search(t) for t in texts))
        stored = [m for t in texts if (m := prof.STORED.search(t))]
        self.assertEqual(len(stored), 2)


class RemainingPathTests(unittest.TestCase):
    def test_without_a_log_group_near_it_the_test_step_is_the_step_apis_own_interval(self):
        log = "\n".join([*LOG.splitlines()[:5], *LOG.splitlines()[6:]])  # the step's own group is gone
        p = prof.profile_job(models.Job.parse(JOB), log, models.NxExecution.report(EXECUTIONS))
        self.assertEqual(
            p.test_step,
            prof.Interval(models.timestamp("2026-11-12T04:01:05Z", "t"), models.timestamp("2026-11-12T04:09:05Z", "t")),
        )

    def test_a_build_target_is_compile_for_its_whole_interval(self):
        tasks = [
            {
                "task": "onetaskgraph:build",
                "result": "executed",
                "started_at": "2026-11-12T04:01:06.000Z",
                "finished_at": "2026-11-12T04:02:06.000Z",
            },
            {
                "task": "onetaskgraph:coverage-build",
                "result": "executed",
                "started_at": "2026-11-12T04:02:06.000Z",
                "finished_at": "2026-11-12T04:02:36.000Z",
            },
        ]
        lines = LOG.splitlines()
        steps_only = "\n".join([*lines[:6], *lines[-2:]])  # the step groups and no task header
        p = prof.profile_job(models.Job.parse(JOB), steps_only, models.NxExecution.report({"tasks": tasks}))
        self.assertAlmostEqual(p.buckets[prof.Bucket.COMPILE_PLAIN], 60, places=3)
        self.assertAlmostEqual(p.buckets[prof.Bucket.COMPILE_INSTRUMENTED], 30, places=3)

    def test_a_process_whose_command_line_cannot_be_read_is_classified_by_its_name(self):
        table = FakeProcessTable()
        table.processes = [FakeProcess(60, 1, "node", 0.0, 0.0, unreadable_command=True)]
        taken = sampler.sample(table, {}, me=999, previous=100.0, tick=1.0)
        self.assertEqual(taken.alive[60].category, models.Category.NODE)

    def test_help_an_explicit_format_and_a_nextest_mode_pass_through(self):
        root = scratch(self)
        binary = RunnerPassthroughTests().binary(root)
        record = root / "tests.jsonl"
        for args, env in (
            (["--help"], {}),
            (["--format=pretty"], {}),
            (["-q"], {"NEXTEST_EXECUTION_MODE": "process-per-test"}),
        ):
            proc = command("testrunner.py", str(binary), *args, env={"SPIKE_TEST_RECORD": str(record), **env})
            self.assertEqual((proc.returncode, proc.stdout.strip()), (7, "args=" + " ".join(args)))
        self.assertFalse(record.exists())
        self.assertEqual(len((root / "tests.jsonl.spawns").read_text().splitlines()), 3)


WORKFLOWS = HERE / "data/workflows"


class LatestPathTests(unittest.TestCase):
    def test_a_date_only_window_is_measured_as_whole_days(self):
        root = scratch(self)
        query = (
            "actions/workflows/ci.yml/runs?event=pull_request&created=2026-11-01T00:00:00Z..2026-11-02T00:00:00Z"
            "&per_page=100&page=1"
        )
        env = fake_gh(root, remaining=4000, responses={query: {"workflow_runs": []}})
        proc = command("ciprofile.py", "durations", "2026-11-01", "2026-11-02", env=env)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("created 2026-11-01T00:00:00Z..2026-11-02T00:00:00Z: 0", proc.stdout)

    def test_an_empty_pr_list_leaves_an_empty_history_not_a_stale_one(self):
        root = scratch(self)
        cache = fixture_cache(root)
        listing = cache / "lists/event_pull_request_created_>_2026-11-01.1.json"
        listing.parent.mkdir(parents=True)
        listing.write_text(json.dumps({"workflow_runs": [RUN_RECORD]}))
        (root / "prs.json").write_text("[]")
        out = root / "h.json"
        out.write_text('{"stale": true}')
        proc = command(
            "ciprofile.py",
            "history",
            str(root / "prs.json"),
            "--out",
            str(out),
            env={"CIPROFILE_CACHE": str(cache), "CIPROFILE_OFFLINE": "1"},
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(json.loads(out.read_text()), {})

    def test_3956s_workflow_and_its_parents_take_the_same_edits(self):
        for workflow in sorted(WORKFLOWS.glob("ci-*.yml")):
            copy = scratch(self) / "ci.yml"
            shutil.copy(workflow, copy)
            proc = command("measurement_only.py", str(copy), "--exclude=x")
            self.assertEqual(proc.returncode, 0, f"{workflow.name}: {proc.stderr}")
            text = copy.read_text()
            self.assertNotIn("save-cache: ${{", text)
            self.assertNotRegex(text, r"CACHE_READ_ONLY: (?!'1')")
            self.assertEqual(text.count("spike/ci-profile/sampler.py"), 3, workflow.name)

    def test_an_extraction_interrupted_part_way_is_replaced_whole(self):
        root = scratch(self)
        env = ClientTests().artifact_gh(root, {"tests.jsonl": b"fresh\n"})
        directory = root / f"cache/runs/{RUN}/artifacts/spike-profile-test-macos"
        directory.mkdir(parents=True)
        (directory / "tests.jsonl").write_bytes(b"half")
        (directory / "stale.json").write_bytes(b"{}")
        with unittest.mock.patch.dict("os.environ", env):
            gh = ghcache.GitHub(cache=root / "cache", offline=False)
            self.assertEqual(
                asyncio.run(gh.artifact_member(models.RunId(RUN), "spike-profile-test-macos", "tests.jsonl")),
                b"fresh\n",
            )
        self.assertFalse((directory / "stale.json").exists())

    def test_a_tick_straddling_the_window_is_credited_only_its_overlap(self):
        busy = {models.Category.COMPILE: 1}
        records = [
            models.Tick(10.5, 1.0, busy, {models.Category.COMPILE: 1.0}),
            models.Tick(20.25, 1.0, busy, {models.Category.COMPILE: 1.0}),
        ]
        division = prof.by_process(records, prof.Interval(10.0, 19.5))
        self.assertAlmostEqual(division.seconds["compile"], 0.5 + 0.25)  # (9.5, 10.5] and (19.25, 20.25] each cut

    def test_an_unwritable_replay_report_exits_2(self):
        root = scratch(self)
        ReplayRefusalTests().history_with(root, "3990", "c:test")
        (root / "p.json").write_text(json.dumps([{"pr": 3990, "files": []}]))
        proc = command(
            "replay.py",
            str(root / "h.json"),
            str(root / "p.json"),
            str(root / "after"),
            str(root / "after"),
            "--json",
            str(root / "no/such/dir/r.json"),
        )
        self.assertEqual(proc.returncode, 2)
        self.assertIn("FileNotFoundError", proc.stderr)

    def test_a_time_or_a_pull_request_list_that_is_falsy_but_not_null_is_refused(self):
        with self.assertRaisesRegex(models.Invalid, "not an ISO-8601 time"):
            models.Job.parse({**JOB, "completed_at": 0})
        with self.assertRaisesRegex(models.Invalid, "not an ISO-8601 time"):
            models.Job.parse({**JOB, "steps": [{**STEPS[0], "started_at": ""}]})
        with self.assertRaisesRegex(models.Invalid, "expected a list"):
            models.Run.parse({**RUN_RECORD, "pull_requests": {}})
        self.assertIsNone(models.Step.parse({"name": "x", "started_at": None, "completed_at": None}, "s"))

    def test_identifiers_must_be_positive(self):
        with self.assertRaisesRegex(models.Invalid, "positive"):
            models.Job.parse({**JOB, "id": 0})
        with self.assertRaisesRegex(models.Invalid, "positive"):
            models.Run.parse({**RUN_RECORD, "pull_requests": [{"number": -1}]})
