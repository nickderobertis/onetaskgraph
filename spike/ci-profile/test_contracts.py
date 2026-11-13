"""Each record the harness shares with another program, held to that program; and the command paths
`test_ciprofile.py` leaves, driven through the real entry points with a stand-in `gh` on PATH.
"""

from __future__ import annotations

import asyncio
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

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

# E402: the modules under test sit beside this file and are found through the sys.path entry above.
import ghcache  # noqa: E402
import jobprofile as prof  # noqa: E402
import models  # noqa: E402
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
        page = [{**JOB, "id": i} for i in range(100)]
        env = fake_gh(
            root,
            remaining=4000,
            responses={
                f"actions/runs/{RUN}/jobs?per_page=100&page=1": {"total_count": 101, "jobs": page},
                f"actions/runs/{RUN}/jobs?per_page=100&page=2": {"total_count": 101, "jobs": [{**JOB, "id": 100}]},
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
                    {"type": "suite", "event": "failed", "binary": "e2e-0123456789abcdef", "exec_time": 9.0},
                ]
            )
        )
        env = {"CIPROFILE_CACHE": str(cache), "CIPROFILE_OFFLINE": "1"}
        proc = command("ciprofile.py", "profile", str(RUN), "--jobs", "test-macos", "--top", "1", env=env)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("9.0s  e2e-0123456789abcdef::broke", proc.stdout)
        self.assertIn("2.5s  instrumented:x::e2e::t2", proc.stdout)
        binaries = proc.stdout.split("test binaries (2, seconds):\n")[1].split("  slowest")[0].strip().splitlines()
        self.assertEqual(len(binaries), 1)  # --top 1
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
