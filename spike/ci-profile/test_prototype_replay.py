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
    def test_a_changed_task_on_the_chain_moves_the_job_end_until_another_task_ends_last(self):
        timeline = {"build": (0.0, 100.0), "e2e": (100.0, 700.0), "other": (50.0, 600.0)}
        self.assertEqual(critical_path.latest_chain(timeline), ["build", "e2e"])
        self.assertEqual(critical_path.predicted_shortening(timeline, {"e2e": 60.0}), 60.0)
        self.assertEqual(critical_path.predicted_shortening(timeline, {"e2e": 300.0}), 100.0)  # `other` ends at 600
        self.assertEqual(critical_path.predicted_shortening(timeline, {"e2e": -30.0}), -30.0)

    def test_a_changed_task_off_the_chain_shortens_nothing_unless_it_grows_past_the_end(self):
        timeline = {"build": (0.0, 100.0), "e2e": (100.0, 700.0), "other": (50.0, 600.0)}
        self.assertEqual(critical_path.predicted_shortening(timeline, {"other": 200.0}), 0.0)
        self.assertEqual(critical_path.predicted_shortening(timeline, {"other": -150.0}), -50.0)

    def test_a_task_the_run_did_not_have_runs_beside_the_gate_from_its_start(self):
        timeline = {"build": (0.0, 100.0), "core": (100.0, 300.0)}
        self.assertEqual(critical_path.predicted_shortening(timeline, {"core": 10.0, "template:test": -50.0}), 10.0)
        self.assertEqual(critical_path.predicted_shortening(timeline, {"core": 10.0, "template:test": -400.0}), -100.0)


class CommandTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="prototype-replay-"))
        self.addCleanup(shutil.rmtree, self.root, True)

    def deltas(
        self, side: str, control: dict, prototype: dict, extra: dict | None = None
    ) -> subprocess.CompletedProcess:
        write_summary(self.root, 1, control, 1800.0)
        write_summary(self.root, 2, control, 1900.0)
        write_summary(self.root, 3, {**prototype, **(extra or {})}, 1300.0)
        write_summary(self.root, 4, prototype, 1300.0)
        return command(
            "deltas", str(self.root), "--control", "1,2", "--side", f"{side}=3,4", "--out", str(self.root / "d.json")
        )

    def test_elapsed_change_wall_share_and_summed_test_seconds_are_kept_apart(self):
        control = {"build": (0, 100, 100), "onetaskgraph-e2e:test": (100, 700, 300)}
        prototype = {"build": (0, 100, 100), "onetaskgraph-e2e:test": (100, 640, 200)}
        proc = self.deltas("pyramid", control, prototype)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        job = json.loads((self.root / "d.json").read_text())["sides"]["pyramid"]["jobs"]["test-macos"]
        self.assertEqual(job["elapsed_saved_s"], {"build": 0.0, "onetaskgraph-e2e:test": 60.0})
        self.assertEqual(job["share_saved_s"]["onetaskgraph-e2e:test"], 100.0)
        self.assertEqual(job["work_saved_s"], {"onetaskgraph-e2e": 550.0})
        self.assertEqual((job["control"]["chain_span_s"], job["side"]["chain_span_s"]), (700.0, 640.0))
        self.assertEqual((job["control"]["gate_span_s"], job["side"]["gate_span_s"]), (700.0, 640.0))

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

    def test_the_replay_predicts_each_prs_shortening_from_its_own_critical_path(self):
        control = {"build": (0, 100, 100), "onetaskgraph-e2e:test": (100, 700, 300)}
        self.assertEqual(
            self.deltas("pyramid", control, {**control, "onetaskgraph-e2e:test": (100, 640, 200)}).returncode, 0
        )
        proc = command(
            "replay", str(DATA / "history-first-runs.json"), str(DATA / "prs30.json"), str(self.root / "d.json")
        )
        self.assertEqual(proc.returncode, 0, proc.stderr)
        line = next(x for x in proc.stdout.splitlines() if "test-macos" in x)
        found = re.search(r"predicted shortening ([+-][0-9.]+) min/PR over 30,", line)
        assert found is not None, line
        predicted = float(found.group(1))
        history = json.loads((DATA / "history-first-runs.json").read_text())
        ran = [
            run["jobs"]["test-macos"]["tasks"]["onetaskgraph-e2e:test"]
            for run in history.values()
            if run["jobs"].get("test-macos", {}).get("tasks", {}).get("onetaskgraph-e2e:test", {}).get("result")
            == "executed"
        ]
        # At most a minute on each PR that ran the task, and less wherever it was off the chain.
        self.assertGreater(predicted, 0)
        self.assertLess(predicted, round(len(ran) / 30, 2))
        self.assertIn("(0 without intervals)", line)
        self.assertIn("measured: wall  30.0 ->  30.1 min, chain  11.7 ->  10.7 min", line)


if __name__ == "__main__":
    unittest.main()
