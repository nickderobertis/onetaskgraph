"""The CI profile harness, driven through each command's real entry point.

Run: uv run --no-project --with psutil==7.1.3 python -m unittest discover -s spike/ci-profile -p 'test_*.py'

GitHub is the one boundary stood in for: a `gh` executable first on PATH answers the reads the
harness makes, the way the real `gh api` does (or the read cache holds them already, offline).
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import textwrap
import time
import unittest
from collections.abc import Mapping
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

# E402: the modules under test sit beside this file and are found through the sys.path entry above.
import jobprofile as prof  # noqa: E402
import models  # noqa: E402
import replay  # noqa: E402
import sampler  # noqa: E402
from models import Category, Exit, Invalid, Job, NxExecution, Tick, sampler_record  # noqa: E402

RUN, JOB_ID = 77, 5

STEPS: list[dict[str, str]] = [
    {
        "name": "Set up job",
        "started_at": "2026-11-12T04:00:00Z",
        "completed_at": "2026-11-12T04:00:10Z",
        "conclusion": "success",
    },
    {
        "name": "Run actions-rust-lang/setup-rust-toolchain@v1",
        "started_at": "2026-11-12T04:00:10Z",
        "completed_at": "2026-11-12T04:01:00Z",
        "conclusion": "success",
    },
    {
        "name": "Run op-nx/github-cache/start-cache-server@abc",
        "started_at": "2026-11-12T04:00:59Z",
        "completed_at": "2026-11-12T04:09:30Z",
        "conclusion": "cancelled",
    },
    {
        "name": "Run actions/cache/restore@v6",
        "started_at": "2026-11-12T04:01:00Z",
        "completed_at": "2026-11-12T04:01:05Z",
        "conclusion": "success",
    },
    {
        "name": "Run actions/cache/save@v6",
        "started_at": "2026-11-12T04:01:05Z",
        "completed_at": "2026-11-12T04:01:05Z",
        "conclusion": "skipped",
    },
    {
        "name": "Test the affected projects",
        "started_at": "2026-11-12T04:01:05Z",
        "completed_at": "2026-11-12T04:09:05Z",
        "conclusion": "success",
    },
    {
        "name": "Post Run astral-sh/setup-uv@v7",
        "started_at": "2026-11-12T04:09:05Z",
        "completed_at": "2026-11-12T04:09:35Z",
        "conclusion": "success",
    },
    {
        "name": "Complete job",
        "started_at": "2026-11-12T04:09:35Z",
        "completed_at": "2026-11-12T04:10:00Z",
        "conclusion": "success",
    },
]
JOB = {
    "id": JOB_ID,
    "name": "test-macos",
    "conclusion": "success",
    "started_at": "2026-11-12T04:00:00Z",
    "completed_at": "2026-11-12T04:10:00Z",
    "steps": STEPS,
}
LOG = "\n".join(
    [
        "2026-11-12T04:00:10.0000000Z ##[group]Run actions-rust-lang/setup-rust-toolchain@v1",
        "2026-11-12T04:00:20.0000000Z ##[group]Run Swatinem/rust-cache@abc",
        "2026-11-12T04:00:25.0000000Z Cache Size: ~1234 MB",
        '2026-11-12T04:00:50.0000000Z Restored from cache key "v0-rust-test-macos"',
        "2026-11-12T04:01:00.0000000Z ##[group]Run actions/cache/restore@v6",
        "2026-11-12T04:01:05.0000000Z ##[group]Run just ci-test-macos",
        "2026-11-12T04:01:06.0000000Z [test,budgets] 🔁 > nx run a-crate:test  [remote cache]",
        "2026-11-12T04:01:07.0000000Z [test,budgets] > nx run b-crate:test",
        "2026-11-12T04:01:08.0000000Z [test,budgets] b-crate:     Blocking waiting for file lock on build directory",
        "2026-11-12T04:01:38.0000000Z [test,budgets] b-crate:    Compiling b-crate v0.1.0",
        "2026-11-12T04:03:38.0000000Z [test,budgets] b-crate:     Finished `test` profile [unoptimized] in 2m",
        "2026-11-12T04:04:07.0000000Z [test,budgets] b-crate: running 3 tests",
        "2026-11-12T04:05:07.0000000Z [test,budgets] b-crate: test result: ok. 3 passed; finished in 60.00s",
        "2026-11-12T04:09:05.0000000Z ##[group]Run node scripts/nx-remote-cache.mjs report",
        "2026-11-12T04:09:06.0000000Z Nx cache: stored 2 entries, 100 bytes; replayed 1 entries",
    ]
)
EXECUTIONS = {
    "tasks": [
        {
            "task": "a-crate:test",
            "result": "remote_hit",
            "started_at": "2026-11-12T04:01:06.000Z",
            "finished_at": "2026-11-12T04:01:07.000Z",
        },
        {
            "task": "b-crate:test",
            "result": "executed",
            "started_at": "2026-11-12T04:01:07.000Z",
            "finished_at": "2026-11-12T04:05:07.000Z",
        },
        {
            "task": "c-crate:test",
            "result": "failed",
            "started_at": "2026-11-12T04:03:07.000Z",
            "finished_at": "2026-11-12T04:05:07.000Z",
        },
    ]
}
RUN_RECORD = {
    "id": RUN,
    "head_sha": "a" * 40,
    "head_branch": "topic",
    "event": "pull_request",
    "created_at": "2026-11-12T04:00:00Z",
    "conclusion": "success",
    "pull_requests": [{"number": 9}],
}


def scratch(case: unittest.TestCase) -> Path:
    path = Path(tempfile.mkdtemp())
    case.addCleanup(shutil.rmtree, path)
    return path


def at(text: str) -> float:
    return prof.timestamp(text, "fixture")


def fixture_cache(root: Path) -> Path:
    cache = root / "cache"
    run = cache / f"runs/{RUN}"
    (run / "logs").mkdir(parents=True)
    (run / "run.json").write_text(json.dumps(RUN_RECORD))
    (run / "jobs.json").write_text(json.dumps({"jobs": [JOB]}))
    (run / f"logs/{JOB_ID}.txt").write_text(LOG)
    executions = run / "artifacts/nx-cache-entries-test-macos"
    executions.mkdir(parents=True)
    (executions / "executions.json").write_text(json.dumps(EXECUTIONS))
    (executions / ".complete").write_text("present\n")  # as the client marks an extracted artifact
    records = run / "artifacts/spike-profile-test-macos"
    records.mkdir(parents=True)
    (records / ".complete").write_text("present\n")
    (records / "tests.jsonl").write_text(
        "\n".join(
            json.dumps(e)
            for e in [
                {
                    "type": "test",
                    "event": "ok",
                    "binary": "e2e-0123456789abcdef",
                    "name": "journey::slow",
                    "exec_time": 41.0,
                },
                {
                    "type": "suite",
                    "event": "ok",
                    "binary": "e2e-0123456789abcdef",
                    "passed": 0,
                    "failed": 0,
                    "ignored": 0,
                    "filtered_out": 0,
                    "exec_time": 42.0,
                },
            ]
        )
    )
    (records / "samples.jsonl").write_text(
        "\n".join(
            json.dumps(e)
            for e in [
                {"kind": "start", "t": at("2026-11-12T04:01:05Z")},
                {
                    "kind": "tick",
                    "t": at("2026-11-12T04:02:00Z"),
                    "dt": 1.0,
                    "leaves": {"compile": 1},
                    "cpu": {"compile": 1.0},
                },
                {"kind": "tick", "t": at("2026-11-12T04:04:30Z"), "dt": 1.0, "leaves": {"test-binary": 1}, "cpu": {}},
                {
                    "kind": "exit",
                    "category": "cli-under-test",
                    "start": at("2026-11-12T04:04:30Z"),
                    "end": at("2026-11-12T04:04:31Z"),
                    "cpu": 0.1,
                },
            ]
        )
    )
    return cache


def command(script: str, *args: str, env: dict | None = None) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(HERE / script), *args],
        capture_output=True,
        text=True,
        env={**os.environ, **(env or {})},
        check=False,
    )


def fake_gh(root: Path, remaining: int, responses: Mapping[str, object], graphql: int = 4000) -> dict:
    """A `gh` first on PATH that answers `api rate_limit` and the given `api` paths, and logs each call."""
    bin_dir = root / "bin"
    bin_dir.mkdir()
    (root / "responses.json").write_text(json.dumps(responses))
    script = bin_dir / "gh"
    script.write_text(
        textwrap.dedent(f"""\
        #!{sys.executable}
        import json, sys
        args = [a for a in sys.argv[1:] if a != "--allow-escape-sequences"]
        with open({str(root / "calls.log")!r}, "a") as log:
            log.write(" ".join(args) + "\\n")
        if args == ["api", "rate_limit"]:
            core, graphql = {{"remaining": {remaining}}}, {{"remaining": {graphql}}}
            print(json.dumps({{"resources": {{"core": core, "graphql": graphql}}}}))
            sys.exit(0)
        responses = json.load(open({str(root / "responses.json")!r}))
        path = args[1].split("repos/nickderobertis/onetaskgraph/", 1)[1]
        if path not in responses:
            print("HTTP 404: " + path, file=sys.stderr)
            sys.exit(1)
        print(json.dumps(responses[path]))
        """)
    )
    script.chmod(0o755)
    return {"PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}", "CIPROFILE_CACHE": str(root / "cache")}


class ProfileAccountingTests(unittest.TestCase):
    def test_every_second_of_the_job_is_counted_once(self):
        p = prof.profile_job(Job.parse(JOB), LOG, NxExecution.report(EXECUTIONS))
        self.assertAlmostEqual(sum(p.buckets.values()), p.wall, places=6)
        self.assertAlmostEqual(
            p.task_share[models.TaskId("b-crate:test")], 120 + 60, places=3
        )  # 04:03:07–04:05:07 shared with c
        self.assertAlmostEqual(p.task_share[models.TaskId("c-crate:test")], 60, places=3)
        self.assertAlmostEqual(p.buckets[prof.Bucket.LOCK_WAIT], 30, places=3)
        self.assertGreater(p.buckets[prof.Bucket.COMPILE_PLAIN], 0)
        self.assertEqual(p.buckets[prof.Bucket.NX_RESTORE], 1)
        assert p.rust_cache is not None
        self.assertEqual((p.rust_cache.size, p.rust_cache.hit), ("1234 MB", True))
        self.assertEqual(p.stored, prof.Stored(2, 100, 1))
        self.assertAlmostEqual(p.buckets[prof.Bucket.CACHE_SAVE], 30, places=3)
        # The background sidecar step overlaps everything and owns no second.
        self.assertLess(p.buckets[prof.Bucket.SETUP], 60)
        self.assertEqual([t.id for t in p.latest_chain], ["a-crate:test", "b-crate:test"])

    def test_the_log_alone_places_tasks_when_the_report_is_absent(self):
        p = prof.profile_job(Job.parse(JOB), LOG, None)
        self.assertEqual(
            {t.id: t.result.value for t in p.tasks}, {"a-crate:test": "remote_hit", "b-crate:test": "executed"}
        )
        self.assertAlmostEqual(sum(p.buckets.values()), p.wall, places=6)

    def test_records_of_the_wrong_shape_are_refused_naming_what(self):
        with self.assertRaisesRegex(Invalid, "before it starts"):
            Job.parse({**JOB, "completed_at": "2026-11-12T03:00:00Z"})
        with self.assertRaisesRegex(Invalid, "unknown Nx task result"):
            NxExecution.report({"tasks": [{**EXECUTIONS["tasks"][0], "result": "maybe"}]})
        with self.assertRaisesRegex(Invalid, "unknown shape"):
            sampler_record({"kind": "tick"})
        with self.assertRaisesRegex(Invalid, "unknown process category"):
            sampler_record({"kind": "exit", "category": "x", "start": 1, "end": 2, "cpu": 0})


class ProfileCommandTests(unittest.TestCase):
    def test_one_run_profiles_offline_from_the_cache(self):
        cache = fixture_cache(scratch(self))
        proc = command(
            "ciprofile.py",
            "profile",
            str(RUN),
            "--jobs",
            "test-macos",
            env={"CIPROFILE_CACHE": str(cache), "CIPROFILE_OFFLINE": "1"},
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        for text in (
            "== test-macos (job 5, success) wall 10.00 min; Nx tasks: 2 run, 1 replayed [executions artifact]",
            "wait:cargo-file-lock",
            "rust-cache restore: 40.0 s, 1234 MB, hit",
            "sampler: test step 0.03 min by leaf process category",
            "cli-under-test            1       1.0      0.1",
            "e2e-0123456789abcdef",
            "41.0s  e2e-0123456789abcdef::journey::slow",
            "(GitHub reads this invocation: 0)",
        ):
            self.assertIn(text, proc.stdout)

    def test_a_job_the_run_did_not_finish_exits_4_naming_it(self):
        cache = fixture_cache(scratch(self))
        proc = command(
            "ciprofile.py", "profile", str(RUN), env={"CIPROFILE_CACHE": str(cache), "CIPROFILE_OFFLINE": "1"}
        )
        self.assertEqual(proc.returncode, 4)
        self.assertIn("has no finished test-linux, test-windows", proc.stderr)

    def test_usage_errors_exit_2(self):
        for args in (["profile", "0"], ["profile", "1", "--jobs", "lint"], ["durations", "yesterday", "now"]):
            self.assertEqual(command("ciprofile.py", *args).returncode, 2, args)

    def test_a_read_below_the_rest_floor_is_refused_before_it_is_made(self):
        root = scratch(self)
        env = fake_gh(root, remaining=100, responses={f"actions/runs/{RUN}": RUN_RECORD})
        proc = command("ciprofile.py", "profile", str(RUN), env=env)
        self.assertEqual(proc.returncode, 3)
        self.assertIn("only 100 REST core requests remain", proc.stderr)
        self.assertEqual((root / "calls.log").read_text().splitlines(), ["api rate_limit"])

    def test_durations_read_each_run_once_and_page_its_jobs(self):
        root = scratch(self)
        query = (
            "actions/workflows/ci.yml/runs?event=pull_request&created=2026-11-01T00:00:00Z..2026-11-02T00:00:00Z"
            "&per_page=100&page=1"
        )
        job = {**JOB, "name": "test-linux"}
        env = fake_gh(
            root,
            remaining=4000,
            responses={
                query: {"workflow_runs": [RUN_RECORD, {**RUN_RECORD, "id": 78}]},
                f"actions/runs/{RUN}/jobs?per_page=100&page=1": {"total_count": 1, "jobs": [job]},
                "actions/runs/78/jobs?per_page=100&page=1": {
                    "total_count": 1,
                    "jobs": [{**job, "completed_at": "2026-11-12T04:20:00Z"}],
                },
            },
        )
        for _ in range(2):
            proc = command("ciprofile.py", "durations", "2026-11-01T00:00:00Z", "2026-11-02T00:00:00Z", env=env)
            self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("test-linux    n=  2 median  15.0 min", proc.stdout)
        reads = [c for c in (root / "calls.log").read_text().splitlines() if c != "api rate_limit"]
        self.assertEqual(len(reads), 3)  # the second invocation answered from the cache

    def test_history_takes_each_prs_first_run(self):
        root = scratch(self)
        cache = fixture_cache(root)
        listing = cache / "lists/event_pull_request_created_>_2026-11-01.1.json"
        listing.parent.mkdir(parents=True)
        listing.write_text(json.dumps({"workflow_runs": [RUN_RECORD]}))
        prs = root / "prs.json"
        prs.write_text(json.dumps([{"pr": 9, "head": "b" * 40, "branch": "topic"}]))
        out = root / "history.json"
        proc = command(
            "ciprofile.py",
            "history",
            str(prs),
            "--out",
            str(out),
            env={"CIPROFILE_CACHE": str(cache), "CIPROFILE_OFFLINE": "1"},
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("#9 run 77 (success): test-macos: 2 run/1 replayed 10.0m", proc.stdout)
        self.assertEqual(json.loads(out.read_text())["9"]["jobs"]["test-macos"]["source"], "executions")


class ReplayCommandTests(unittest.TestCase):
    def setUp(self):
        self.root = scratch(self)
        (self.root / "after").mkdir()
        (self.root / "before").mkdir()
        (self.root / "after/onetaskgraph-e2e__test.json").write_text(
            json.dumps({"files": ["crates/onetaskgraph-core/tests/engine.rs", "biome.json"]})
        )
        (self.root / "after/scripts__distribution-test.json").write_text(json.dumps({"files": []}))
        (self.root / "before/scripts__distribution-test.json").write_text(json.dumps({"files": []}))
        history = {
            "4000": {
                "run": 1,
                "head_sha": "c" * 40,
                "created_at": "2026-11-12T04:00:00Z",
                "conclusion": "success",
                "runs_of_pr": 1,
                "jobs": {
                    "test-macos": {
                        "job_id": 1,
                        "source": "executions",
                        "wall_s": 600,
                        "conclusion": "success",
                        "tasks": {
                            "onetaskgraph-e2e:test": {"result": "executed", "seconds": 300, "share_s": 120},
                            "scripts:distribution-test": {"result": "executed", "seconds": 400, "share_s": 240},
                            "onetaskgraph-core:test": {"result": "remote_hit", "seconds": 1, "share_s": 1},
                        },
                    }
                },
            }
        }
        (self.root / "history.json").write_text(json.dumps(history))
        (self.root / "prs.json").write_text(
            json.dumps([{"pr": 4000, "files": ["crates/onetaskgraph-core/tests/engine.rs"]}])
        )

    def run_replay(self, *extra):
        r = self.root
        return command(
            "replay.py", str(r / "history.json"), str(r / "prs.json"), str(r / "after"), str(r / "before"), *extra
        )

    def test_a_dependency_test_edit_and_a_test_only_diff_would_have_replayed(self):
        proc = self.run_replay("--json", str(self.root / "out.json"))
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("test-macos           2         1    10.0 min", proc.stdout)
        saved = json.loads((self.root / "out.json").read_text())["saved_s"]
        self.assertEqual(saved["dependents-skip-dependency-tests"]["test-macos"], 120)
        self.assertEqual(saved["distribution-only-when-publishable-changes"]["test-macos"], 240)

    def test_an_input_of_the_wrong_shape_exits_2_naming_it(self):
        (self.root / "prs.json").write_text(json.dumps([{"pr": 4000, "files": "crates/x"}]))
        proc = self.run_replay()
        self.assertEqual(proc.returncode, 2)
        self.assertIn("expected a list", proc.stderr)

    def test_the_rules_take_only_what_they_name(self):
        e2e = replay.TaskId("onetaskgraph-e2e:test")
        none = replay.Publishable(frozenset())
        diff = replay.Diff(frozenset(), none)
        self.assertTrue(diff.replays(replay.Option.NON_SEMANTIC_GLOBALS, e2e, ["biome.json"]))
        self.assertFalse(
            diff.replays(replay.Option.NON_SEMANTIC_GLOBALS, e2e, ["biome.json", "crates/onetaskgraph-core/src/lib.rs"])
        )
        self.assertFalse(
            diff.replays(replay.Option.DEPENDENCY_TESTS, e2e, ["crates/onetaskgraph-e2e/tests/e2e/main.rs"])
        )
        distribution = replay.TaskId("scripts:distribution-test")
        source_edit = replay.Diff(frozenset({"crates/onetaskgraph-core/src/lib.rs"}), none)
        self.assertFalse(source_edit.replays(replay.Option.DISTRIBUTION_WHEN_PUBLISHABLE, distribution, []))
        script_edit = replay.Diff(frozenset({"scripts/x.sh"}), replay.Publishable(frozenset({"scripts/x.sh"})))
        self.assertFalse(script_edit.replays(replay.Option.DISTRIBUTION_WHEN_PUBLISHABLE, distribution, []))
        self.assertEqual(
            replay.most_specific({replay.InputKind.DEP_SRC, replay.InputKind.OWN_TESTS}), replay.InputKind.OWN_TESTS
        )

    def test_what_the_distribution_journeys_read_is_followed_through_their_scripts(self):
        root = scratch(self)
        (root / "scripts").mkdir()
        (root / "scripts/project.json").write_text(
            json.dumps(
                {
                    "targets": {
                        "distribution-check": {"options": {"commands": ["bash scripts/check.sh"]}},
                        "distribution-test": {"options": {"command": "bash scripts/test.sh"}},
                    }
                }
            )
        )
        (root / "scripts/check.sh").write_text("python3 scripts/helper.py\n")
        (root / "scripts/helper.py").write_text("open('release-targets.toml')\n")
        (root / "scripts/test.sh").write_text("echo missing/never.sh\n")
        (root / "release-targets.toml").write_text("")
        reads = replay.distribution_named_files(root)
        self.assertEqual(reads, {"scripts/check.sh", "scripts/helper.py", "scripts/test.sh", "release-targets.toml"})
        # Against this repository: every file the real distribution scripts name is publishable.
        real = replay.Publishable(replay.distribution_named_files(replay.REPO_ROOT))
        for script in ("scripts/test-distribution.sh", "scripts/check-distribution-contract.sh"):
            text = (replay.REPO_ROOT / script).read_text()
            for named in replay.NAMED_FILE.findall(text):
                path = named.removeprefix("./")
                if (replay.REPO_ROOT / path).is_file():
                    self.assertIn(path, real, f"{script} names {path}")

    def test_paths_and_input_files_are_held_to_what_they_claim(self):
        with self.assertRaisesRegex(Invalid, "repository-relative"):
            replay.parse_prs([{"pr": 1, "files": ["/etc/passwd"]}])
        with self.assertRaisesRegex(Invalid, "normalized"):
            replay.parse_prs([{"pr": 1, "files": ["crates/../x"]}])
        (self.root / "after/onetaskgraph-e2e__test.json").write_text(
            json.dumps({"project": "onetaskgraph-core", "target": "test", "files": []})
        )
        with self.assertRaisesRegex(Invalid, "holds the inputs of onetaskgraph-core:test"):
            replay.load_inputs(self.root / "after")
        history = json.loads((self.root / "history.json").read_text())
        history["4000"]["jobs"]["test-macos"]["tasks"]["onetaskgraph-e2e:test"]["result"] = "maybe"
        with self.assertRaisesRegex(Invalid, "unknown Nx task result"):
            models.history(history)


class SamplerCommandTests(unittest.TestCase):
    def test_classification_names_what_the_job_spends_on(self):
        for name, cmd, want in [
            ("rustc", [], Category.COMPILE),
            ("ld64", [], Category.LINK),
            ("link.exe", [], Category.LINK),
            ("onetaskgraph.exe", [], Category.CLI_UNDER_TEST),
            ("e2e-0123456789abcdef", [], Category.TEST_BINARY),
            ("node", ["node", "/w/node_modules/nx/bin/nx.js"], Category.NX),
            ("node", ["node", "/a/op-nx/start-cache-server/dist/main.js"], Category.NX_CACHE_SIDECAR),
            ("node", ["node", "x.test.mjs"], Category.NODE),
            ("python3.13", [], Category.PYTHON),
            ("git", [], Category.GIT),
            ("bun", [], Category.BUN),
            ("gtar", [], Category.ARCHIVE),
            ("rustdoc", [], Category.DOCTEST),
            ("cargo-nextest", [], Category.CARGO),
            ("bash", [], Category.SHELL),
            ("llvm-profdata", [], Category.COVERAGE_REPORT),
            ("Xorg", [], Category.OTHER),
        ]:
            self.assertEqual(sampler.classify(name, cmd), want, name)

    def test_the_recorder_records_a_process_it_watched_end(self):
        try:
            import psutil  # noqa: F401, PLC0415  # ty: ignore[unresolved-import] — the recorder's own dependency, absent here is a skip
        except ImportError:
            self.skipTest("psutil is not installed here; a demonstration run installs it beside the recorder")
        out = scratch(self) / "samples.jsonl"
        recorder = subprocess.Popen([sys.executable, str(HERE / "sampler.py"), str(out), "--tick", "0.2"])
        try:
            time.sleep(0.5)
            child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(1.5)"])
            child.wait()
            time.sleep(0.6)
        finally:
            Path(f"{out}.stop").write_text("")
            self.assertEqual(recorder.wait(timeout=60), 0)
        records = [sampler_record(json.loads(line)) for line in out.read_text().splitlines()]
        self.assertGreater(len([r for r in records if isinstance(r, Tick)]), 3)
        ended = [r for r in records if isinstance(r, Exit) and r.pid == child.pid]
        self.assertEqual(
            [r.category for r in ended], [Category.PYTHON], "the child should have been recorded ending once"
        )

    def test_a_tick_that_is_not_a_finite_positive_number_is_a_usage_error(self):
        for tick in ("0", "-1", "nan", "inf"):
            proc = command("sampler.py", "out.jsonl", "--tick", tick)
            self.assertEqual(proc.returncode, 2, tick)
            self.assertIn("--tick takes a finite positive number", proc.stderr)

    def test_an_idle_tick_is_a_wait_and_a_busy_one_is_split_among_its_leaves(self):
        records = [
            Tick(10.0, 1.0, {Category.COMPILE: 2, Category.LINK: 1}, {Category.COMPILE: 2.0, Category.LINK: 1.0}),
            Tick(11.0, 1.0, {Category.TEST_BINARY: 1}, {Category.TEST_BINARY: 0.01}),
            Exit(Category.GIT, 10.2, 10.7, 0.1),
        ]
        division = prof.by_process(records, prof.Interval(0, 100))
        self.assertEqual(division.seconds, {"compile": 0.5, "link": 0.5, "wait:in-tests(network,sleep,poll)": 1.0})
        self.assertEqual(division.exits[Category.GIT], prof.ExitTotals(1, 0.5, 0.1))


class RunnerCommandTests(unittest.TestCase):
    """The runner through its real command: libtest JSON on, and a cargo the test starts sees no RUSTC_BOOTSTRAP."""

    def setUp(self):
        self.tmp = scratch(self)
        self.fake_cargo = self.tmp / "real-cargo"
        self.fake_cargo.write_text(
            "#!/usr/bin/env python3\nimport os, sys\n"
            "print('bootstrap=' + str(os.environ.get('RUSTC_BOOTSTRAP')) + ' args=' + ' '.join(sys.argv[1:]))\n"
        )
        self.fake_cargo.chmod(0o755)

    def run_binary(self, body: str, code: int):
        binary = self.tmp / "suite-0123456789abcdef"
        binary.write_text("#!/usr/bin/env python3\nimport json, os, subprocess, sys\n" + body + f"\nsys.exit({code})\n")
        binary.chmod(0o755)
        record = self.tmp / "spike" / "tests.jsonl"
        record.parent.mkdir()
        proc = command(
            "testrunner.py", str(binary), "-q", env={"SPIKE_TEST_RECORD": str(record), "CARGO": str(self.fake_cargo)}
        )
        return proc, [json.loads(x) for x in record.read_text().splitlines()]

    @unittest.skipUnless(shutil.which("rustc") and os.name != "nt", "compiles the shim with rustc")
    def test_a_cargo_the_test_starts_runs_without_rustc_bootstrap(self):
        proc, events = self.run_binary(
            "assert os.environ['RUSTC_BOOTSTRAP'] == '1' and '--format=json' in sys.argv and '-q' not in sys.argv\n"
            "seen = subprocess.run(['cargo', 'test', '--no-run'], capture_output=True, text=True).stdout.strip()\n"
            "print(json.dumps({'type': 'suite', 'event': 'started', 'test_count': 1}))\n"
            "print(json.dumps({'type': 'test', 'event': 'ok', 'name': 'nested', 'exec_time': 0.5}))\n"
            "print(json.dumps({'type': 'suite', 'event': 'ok', 'passed': 1, 'failed': 0, 'ignored': 0, 'filtered_out': 0, 'exec_time': 0.5}))\n"
            "print(seen, file=sys.stderr)",
            0,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("bootstrap=None args=test --no-run", proc.stderr)
        self.assertEqual([e["exec_time"] for e in events if e.get("type") == "test"], [0.5])
        self.assertIn("test result: ok. 1 passed", proc.stdout)

    @unittest.skipUnless(shutil.which("rustc") and os.name != "nt", "compiles the shim with rustc")
    def test_a_failing_suite_keeps_its_status_and_prints_what_failed(self):
        proc, _events = self.run_binary(
            "print(json.dumps({'type': 'suite', 'event': 'started', 'test_count': 1}))\n"
            "print('not json from the binary')\n"
            "print(json.dumps({'type': 'test', 'event': 'failed', 'name': 't::slow', 'exec_time': 1.5,"
            " 'stdout': 'boom'}))\n"
            "print(json.dumps({'type': 'suite', 'event': 'failed', 'passed': 0, 'failed': 1, 'ignored': 0, 'filtered_out': 0, 'exec_time': 1.5}))",
            101,
        )
        self.assertEqual(proc.returncode, 101)
        for text in ("boom", "not json from the binary", "test result: failed"):
            self.assertIn(text, proc.stdout)

    def test_no_binary_is_a_usage_error_on_standard_error(self):
        proc = command("testrunner.py")
        self.assertEqual(proc.returncode, 2)
        self.assertIn("testrunner.py <test binary>", proc.stderr)


class MeasurementOnlyCommandTests(unittest.TestCase):
    """The rewritten workflow is GitHub-hosted, so it cannot run here: these hold its text to each edit, and the
    hosted demonstration runs the report cites (their cache steps and the recorders' artifacts) are its execution."""

    def run_tool(self, *args):
        workflow = scratch(self) / "ci.yml"
        shutil.copy(HERE.parents[1] / ".github/workflows/ci.yml", workflow)
        proc = command("measurement_only.py", str(workflow), *args)
        return proc, workflow.read_text(encoding="utf-8")

    def test_todays_workflow_writes_no_cache_and_scopes_every_measurement_variable_to_the_test_step(self):
        proc, text = self.run_tool("--exclude=x,y", "--step-env=CARGO_PROFILE_RELEASE_INCREMENTAL=true")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertNotIn("save-cache: ${{", text)
        self.assertNotRegex(text, r"cache-save-if: (?!false)")
        self.assertNotRegex(text, r"CACHE_READ_ONLY: (?!'1')")
        self.assertNotIn("nx-cache-entries-", text)
        for job in ("test-linux", "test-macos", "test-windows"):
            self.assertIn(f"name: spike-profile-{job}", text)
            self.assertIn(f"just ci-{job} --exclude=x,y || status=$?", text)
        for line in [x for x in text.splitlines() if "CARGO_PROFILE_RELEASE_INCREMENTAL" in x or "_RUNNER=" in x]:
            self.assertTrue(line.startswith("          export "), line)
        self.assertEqual(
            text.count("GITHUB_ENV"), (HERE.parents[1] / ".github/workflows/ci.yml").read_text().count("GITHUB_ENV")
        )

    def test_a_step_variable_can_be_held_to_one_job(self):
        proc, text = self.run_tool("--step-env=test-macos:CARGO_PROFILE_DEV_OPT_LEVEL=1")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(text.count("export CARGO_PROFILE_DEV_OPT_LEVEL=1"), 1)
        macos = text.split("\n  test-macos:\n")[1].split("\n  budgets:\n")[0]
        self.assertIn("export CARGO_PROFILE_DEV_OPT_LEVEL=1", macos)

    def test_without_recorders_only_the_cache_edits_remain(self):
        proc, text = self.run_tool("--no-recorders")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertNotIn("sampler.py", text)
        self.assertNotIn("testrunner.py", text)
        self.assertIn("save-cache: false", text)

    def test_shell_text_and_an_unknown_workflow_are_refused(self):
        self.assertEqual(self.run_tool("--exclude=a;$(x)")[0].returncode, 2)
        self.assertEqual(self.run_tool("--step-env=lower=1")[0].returncode, 2)
        other = scratch(self) / "ci.yml"
        other.write_text("name: other\n")
        proc = command("measurement_only.py", str(other))
        self.assertEqual(proc.returncode, 2)
        self.assertIn("not one the spike's edits know", proc.stderr)


if __name__ == "__main__":
    unittest.main()
