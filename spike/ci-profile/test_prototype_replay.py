"""prototype_replay.py: its critical-path rule on hand-built timelines, and both commands through the entry
point, over run summaries shaped as `ciprofile.py profile --json` writes them and the committed 30-PR history."""

from __future__ import annotations

import json
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
DATA = HERE / "data"
sys.path.insert(0, str(HERE))

import critical_path  # noqa: E402


def command(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(HERE / "prototype_replay.py"), *args], capture_output=True, text=True, check=False
    )


def write_summary(
    directory: Path, run: int, tasks: dict[str, tuple[float, float, float]], work: float, replayed: tuple[str, ...] = ()
) -> None:
    """tasks: name -> (start, end, share), each executed; `replayed` tasks are scheduled and replayed."""
    job = {
        "job_id": run,
        "wall_s": 1800 + run,
        "buckets_s": {},
        "tasks": [
            {"task": t, "result": "executed", "start": s, "end": e, "share_s": sh} for t, (s, e, sh) in tasks.items()
        ]
        + [{"task": t, "result": "remote_hit", "start": 0, "end": 0, "share_s": 0} for t in ("x:replayed", *replayed)],
        "binaries_s": {"instrumented:onetaskgraph-e2e::e2e": work, "e2e-0123456789abcdef": 5.0},
    }
    (directory / f"profile-{run}.json").write_text(json.dumps({"run": run, "jobs": {"test-macos": job}}))


class CriticalPathTests(unittest.TestCase):
    """critical_path.py re-times a run over Nx's task dependencies; timestamps alone imply no dependency."""

    def test_tasks_that_ran_one_after_another_without_an_edge_are_independent(self):
        # Y began the moment X ended, but nothing makes Y wait for X: a timeline chain would move Y.
        timeline = {"x:test": (0.0, 100.0), "y:test": (100.0, 300.0)}
        self.assertEqual(critical_path.predicted_shortening(timeline, {}, {"x:test": 50.0}), 0.0)
        # With the edge, Y did wait for X, and moves with it.
        self.assertEqual(critical_path.predicted_shortening(timeline, {"y:test": {"x:test"}}, {"x:test": 50.0}), 50.0)

    def test_shortening_one_path_leaves_a_competing_dependent_path_where_it_was(self):
        # A -> C and D -> B; B starts as A ends, but depends on D, which had ended 5 s earlier.
        timeline = {"a": (0.0, 100.0), "d": (0.0, 95.0), "c": (100.0, 250.0), "b": (100.0, 300.0)}
        deps = {"c": {"a"}, "b": {"d"}}
        self.assertEqual(critical_path.predicted_shortening(timeline, deps, {"a": 50.0}), 0.0)  # B still ends last
        self.assertEqual(critical_path.predicted_shortening(timeline, deps, {"c": 100.0}), 0.0)
        self.assertEqual(critical_path.predicted_shortening(timeline, deps, {"b": 30.0}), 30.0)
        self.assertEqual(critical_path.predicted_shortening(timeline, deps, {"b": 80.0}), 50.0)  # until C ends last
        self.assertEqual(critical_path.predicted_shortening(timeline, deps, {"a": -100.0}), -50.0)  # C now ends at 350

    def test_a_task_that_waited_on_something_else_keeps_its_start_unless_a_dependency_now_ends_later(self):
        # E depends on A, but started 60 s after A ended: a slot, a phase, the runner — not A.
        timeline = {"a": (0.0, 100.0), "e": (160.0, 400.0)}
        self.assertEqual(critical_path.predicted_shortening(timeline, {"e": {"a"}}, {"a": 50.0}), 0.0)
        self.assertEqual(critical_path.predicted_shortening(timeline, {"e": {"a"}}, {"a": -100.0}), -40.0)

    def test_dependencies_are_followed_through_tasks_the_run_did_not_execute(self):
        graph = {"e2e:test": ["cli:build"], "cli:build": ["core:build"], "core:build": []}
        self.assertEqual(
            critical_path.executed_deps(graph, {"e2e:test", "core:build"}),
            {
                "e2e:test": {"core:build"},
                "core:build": set(),
            },
        )

    def test_a_task_the_run_did_not_have_runs_beside_the_gate_from_its_start(self):
        timeline = {"build": (0.0, 100.0), "core": (100.0, 300.0)}
        deps = {"core": {"build"}}
        self.assertEqual(
            critical_path.predicted_shortening(timeline, deps, {"core": 10.0, "template:test": -50.0}), 10.0
        )
        self.assertEqual(
            critical_path.predicted_shortening(timeline, deps, {"core": 10.0, "template:test": -400.0}), -100.0
        )


class CommandTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="prototype-replay-"))
        self.addCleanup(shutil.rmtree, self.root, True)

    def deltas(
        self, side: str, control: dict, prototype: dict, extra: dict | None = None, graphs: Path | None = None
    ) -> subprocess.CompletedProcess:
        write_summary(self.root, 1, control, 1800.0)
        write_summary(self.root, 2, control, 1900.0)
        write_summary(self.root, 3, {**prototype, **(extra or {})}, 1300.0)
        write_summary(self.root, 4, prototype, 1300.0)
        args = [
            "deltas",
            str(self.root),
            "--control",
            "1,2",
            "--side",
            f"{side}=3,4",
            "--out",
            str(self.root / "d.json"),
        ]
        return command(*args, *(["--graphs", str(graphs)] if graphs else []))

    def test_elapsed_change_wall_share_and_summed_test_seconds_are_kept_apart(self):
        control = {"build": (0, 100, 100), "onetaskgraph-e2e:test": (100, 700, 300)}
        prototype = {"build": (0, 100, 100), "onetaskgraph-e2e:test": (100, 640, 200)}
        proc = self.deltas("pyramid", control, prototype)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        job = json.loads((self.root / "d.json").read_text())["sides"]["pyramid"]["jobs"]["test-macos"]
        self.assertEqual(job["elapsed_saved_s"], {"build": 0.0, "onetaskgraph-e2e:test": 60.0})
        self.assertEqual(job["share_saved_s"]["onetaskgraph-e2e:test"], 100.0)
        self.assertEqual(job["work_saved_s"], {"onetaskgraph-e2e": 550.0})
        self.assertEqual((job["control"]["gate_span_s"], job["side"]["gate_span_s"]), (700.0, 640.0))
        self.assertEqual(
            (job["control"]["runs_wall_s"], job["side"]["runs_wall_s"]), ([1801.0, 1802.0], [1803.0, 1804.0])
        )
        self.assertNotIn("model_on_control_s", job)

    def test_a_task_left_out_is_out_of_both_sides_timelines_and_recorded(self):
        control = {"build": (0, 100, 100), "core:test": (100, 400, 200), "scripts:distribution-test": (0, 900, 500)}
        prototype = {**control, "scripts:distribution-test": (0, 60, 50)}  # failed early, for another reason
        self.assertEqual(self.deltas("split", control, prototype).returncode, 0)
        job = json.loads((self.root / "d.json").read_text())["sides"]["split"]["jobs"]["test-macos"]
        self.assertEqual((job["control"]["gate_span_s"], job["side"]["gate_span_s"]), (900.0, 400.0))
        args = ["deltas", str(self.root), "--control", "1,2", "--side", "split=3,4", "--out", str(self.root / "d.json")]
        proc = command(*args, "--leave-out", "test-macos:scripts:distribution-test")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        job = json.loads((self.root / "d.json").read_text())["sides"]["split"]["jobs"]["test-macos"]
        self.assertEqual((job["control"]["gate_span_s"], job["side"]["gate_span_s"]), (400.0, 400.0))
        self.assertEqual(job["left_out"], ["scripts:distribution-test"])
        self.assertNotIn("scripts:distribution-test", job["elapsed_saved_s"])
        self.assertEqual(command(*args, "--leave-out", "nowhere").returncode, 2)

    def test_only_the_named_jobs_are_compared(self):
        tasks = {"build": (0, 100, 100), "core:test": (100, 400, 200)}
        self.assertEqual(self.deltas("split", tasks, tasks).returncode, 0)
        args = ["deltas", str(self.root), "--control", "1,2", "--side", "split=3,4", "--out", str(self.root / "d.json")]
        self.assertEqual(command(*args, "--jobs", "test-linux").returncode, 0)
        self.assertEqual(json.loads((self.root / "d.json").read_text())["sides"]["split"]["jobs"], {})
        self.assertEqual(command(*args, "--jobs", "test-macos").returncode, 0)
        self.assertIn("test-macos", json.loads((self.root / "d.json").read_text())["sides"]["split"]["jobs"])
        self.assertEqual(command(*args, "--jobs", "lint").returncode, 2)

    def test_the_model_is_checked_on_the_control_runs_own_timelines_and_graphs(self):
        control = {"build": (0, 100, 100), "onetaskgraph-e2e:test": (100, 700, 300), "other:test": (0, 650, 300)}
        prototype = {**control, "onetaskgraph-e2e:test": (100, 640, 200)}
        graphs = self.root / "graphs"
        graphs.mkdir()
        for run in (1, 2):
            (graphs / f"run-{run}.json").write_text(json.dumps({"dependencies": {"onetaskgraph-e2e:test": ["build"]}}))
        self.assertEqual(self.deltas("pyramid", control, prototype, graphs=graphs).returncode, 0)
        job = json.loads((self.root / "d.json").read_text())["sides"]["pyramid"]["jobs"]["test-macos"]
        self.assertEqual(job["model_on_control_s"], [50.0, 50.0])  # 60 s sooner, until other:test ends last

    def test_sides_whose_selections_differ_are_refused_naming_the_tasks(self):
        tasks = {"build": (0, 100, 100), "onetaskgraph-core:test": (100, 300, 200)}
        proc = self.deltas("split", tasks, tasks, extra={"onetaskgraph-plugin-api:test": (0, 50, 25)})
        self.assertEqual(proc.returncode, 2)
        self.assertIn("split only ['onetaskgraph-plugin-api:test']", proc.stderr)

    def test_a_declared_task_the_prototype_adds_is_a_cost_and_a_replay_on_one_side_is_recorded_not_compared(self):
        common: dict[str, tuple[float, float, float]] = {
            "build": (0, 100, 100),
            "onetaskgraph-core:test": (100, 400, 200),
        }
        write_summary(self.root, 1, common, 0, replayed=("lib:test",))
        write_summary(self.root, 2, common, 0, replayed=("lib:test",))
        split: dict[str, tuple[float, float, float]] = {
            **common,
            "onetaskgraph-core:test": (100, 390, 190),
            "lib:test": (0, 80, 40),
        }
        write_summary(self.root, 3, {**split, "onetaskgraph-template:coverage": (0, 30, 15)}, 0)
        write_summary(self.root, 4, {**split, "onetaskgraph-template:coverage": (0, 50, 25)}, 0)
        args = ("deltas", str(self.root), "--control", "1,2", "--side", "split=3,4", "--out", str(self.root / "d.json"))
        refused = command(*args)
        self.assertEqual(refused.returncode, 2)
        self.assertIn("split only ['onetaskgraph-template:coverage']", refused.stderr)
        proc = command(*args, "--side-only", "test-macos:onetaskgraph-template:coverage")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        job = json.loads((self.root / "d.json").read_text())["sides"]["split"]["jobs"]["test-macos"]
        self.assertEqual(job["side_only"], ["onetaskgraph-template:coverage"])
        self.assertEqual(job["executed_by_one_side"], {"control": [], "split": ["lib:test"]})
        self.assertEqual(
            job["elapsed_saved_s"],
            {"build": 0.0, "onetaskgraph-core:test": 10.0, "onetaskgraph-template:coverage": -40.0},
        )

    def test_the_replay_estimates_each_prs_shortening_over_its_own_task_graph(self):
        control = {"build": (0, 100, 100), "onetaskgraph-e2e:test": (100, 700, 300)}
        self.assertEqual(
            self.deltas("pyramid", control, {**control, "onetaskgraph-e2e:test": (100, 640, 200)}).returncode, 0
        )
        args = ["replay", str(DATA / "history-first-runs.json"), str(DATA / "prs30.json"), str(self.root / "d.json")]
        self.assertEqual(command(*args).returncode, 2)  # no graphs, no estimate
        proc = command(*args, "--graphs", str(DATA / "taskgraphs"))
        self.assertEqual(proc.returncode, 0, proc.stderr)
        line = next(x for x in proc.stdout.splitlines() if "test-macos" in x)
        found = re.search(r"estimated shortening ([+-][0-9.]+) min/PR over 30,", line)
        assert found is not None, line
        estimated = float(found.group(1))
        history = json.loads((DATA / "history-first-runs.json").read_text())
        ran = sum(
            run["jobs"].get("test-macos", {}).get("tasks", {}).get("onetaskgraph-e2e:test", {}).get("result")
            == "executed"
            for run in history.values()
        )
        # At most a minute on each PR that ran the task, and less wherever something else ended last.
        self.assertGreaterEqual(estimated, 0)
        self.assertLessEqual(estimated, round(ran / 30, 2))
        self.assertIn("(0 without intervals)", line)
        self.assertIn("measured gate span (min) 11.7, 11.7 -> 10.7, 10.7, wall 30.0, 30.0 -> 30.1, 30.1", line)


if __name__ == "__main__":
    unittest.main()
