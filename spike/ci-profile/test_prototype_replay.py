"""prototype_replay.py through its entry point, over the committed 30-PR population and run summaries
shaped as `ciprofile.py profile --json` writes them."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
DATA = HERE / "data"


def command(*args: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, str(HERE / "prototype_replay.py"), *args], capture_output=True, text=True, check=False
    )


def write_summary(directory: Path, run: int, wall: float, shares: dict[str, float], compile_s: float = 0) -> None:
    tasks = [
        {
            "task": t,
            "result": "executed",
            "start": 0,
            "end": s + compile_s,
            "compile_end": compile_s,
            "share_s": s + compile_s,
        }
        for t, s in shares.items()
    ]
    job = {"job_id": run, "wall_s": wall, "buckets_s": {}, "tasks": tasks}
    (directory / f"profile-{run}.json").write_text(
        json.dumps({"run": run, "head_sha": "a" * 40, "jobs": {"test-macos": job}})
    )


class PrototypeReplayTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="prototype-replay-"))
        self.addCleanup(lambda: subprocess.run(["rm", "-rf", str(self.root)], check=False))

    def deltas(self, side: str, control: dict[str, float], prototype: dict[str, float]) -> Path:
        write_summary(self.root, 1, 1800, control)
        write_summary(self.root, 2, 1900, control)
        write_summary(self.root, 3, 1700, prototype)
        write_summary(self.root, 4, 1700, prototype)
        out = self.root / "deltas.json"
        proc = command("deltas", str(self.root), "--control", "1,2", "--side", f"{side}=3,4", "--out", str(out))
        self.assertEqual(proc.returncode, 0, proc.stderr)
        return out

    def replay(self, deltas: Path) -> subprocess.CompletedProcess:
        return command("replay", str(DATA / "history-first-runs.json"), str(DATA / "prs30.json"), str(deltas))

    def test_a_measured_e2e_saving_is_replayed_on_every_pr_whose_run_executed_that_task(self):
        out = self.deltas(
            "pyramid", {"onetaskgraph-e2e:test": 300, "x:y": 50}, {"onetaskgraph-e2e:test": 240, "x:y": 50}
        )
        record = json.loads(out.read_text())["sides"]["pyramid"]["jobs"]["test-macos"]
        self.assertEqual(record["saved_s"], {"onetaskgraph-e2e:test": 60.0, "x:y": 0.0})
        self.assertEqual((record["control_wall_s"], record["side_wall_s"]), (1850.0, 1700.0))
        history = json.loads((DATA / "history-first-runs.json").read_text())
        ran = sum(
            1
            for run in history.values()
            if run["jobs"].get("test-macos", {}).get("tasks", {}).get("onetaskgraph-e2e:test", {}).get("result")
            == "executed"
        )
        proc = self.replay(out)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn(f"replayed saving {60 * ran / 30 / 60:+.2f} min/PR over 30", proc.stdout)

    def test_a_split_saves_only_on_prs_changing_the_engine_outside_what_it_moved(self):
        out = self.deltas(
            "split",
            {"onetaskgraph-core:test": 400},
            {"onetaskgraph-core:test": 340, "onetaskgraph-template:test": 30},
        )
        prs = json.loads((DATA / "prs30.json").read_text())
        history = json.loads((DATA / "history-first-runs.json").read_text())
        expected = 0.0
        for pr in prs:
            files = pr["files"]
            tasks = history[str(pr["pr"])]["jobs"].get("test-macos", {}).get("tasks", {})
            core_ran = tasks.get("onetaskgraph-core:test", {}).get("result") == "executed"
            moved = any(
                f.startswith(("crates/onetaskgraph-core/src/template/", "crates/onetaskgraph-core/tests/templates.rs"))
                or f == "crates/onetaskgraph-core/src/yaml.rs"
                or f.startswith("crates/onetaskgraph-core/tests/fixtures/template-front-matter")
                for f in files
            )
            touches = moved or any(f.startswith("crates/onetaskgraph-core/") for f in files)
            if touches:
                expected += (60 if core_ran else 0) - (30 if moved else 0)
        proc = self.replay(out)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn(f"replayed saving {expected / 30 / 60:+.2f} min/PR over 30", proc.stdout)

    def test_test_run_seconds_leave_out_the_compile_a_task_happened_to_pay_for(self):
        write_summary(self.root, 1, 1800, {"onetaskgraph-core:test": 70}, compile_s=430)
        write_summary(self.root, 2, 1800, {"onetaskgraph-core:test": 70}, compile_s=390)
        write_summary(self.root, 3, 1800, {"onetaskgraph-core:test": 65}, compile_s=100)
        write_summary(self.root, 4, 1800, {"onetaskgraph-core:test": 65}, compile_s=100)
        out = self.root / "deltas.json"
        args = ("deltas", str(self.root), "--control", "1,2", "--side", "split=3,4", "--out", str(out))
        self.assertEqual(command(*args, "--measure", "test-run").returncode, 0)
        record = json.loads(out.read_text())
        self.assertEqual(record["measure"], "test-run")
        self.assertEqual(record["sides"]["split"]["jobs"]["test-macos"]["saved_s"], {"onetaskgraph-core:test": 5.0})
        self.assertEqual(command(*args).returncode, 0)
        by_share = json.loads(out.read_text())["sides"]["split"]["jobs"]["test-macos"]["saved_s"]
        self.assertEqual(by_share["onetaskgraph-core:test"], 315.0)  # the compile the control's task paid for

    def test_an_unknown_side_and_a_missing_summary_are_refused(self):
        proc = command("deltas", str(self.root), "--control", "1", "--side", "faster=2", "--out", str(self.root / "o"))
        self.assertEqual(proc.returncode, 2)
        proc = command("deltas", str(self.root), "--control", "9", "--side", "split=8", "--out", str(self.root / "o"))
        self.assertEqual(proc.returncode, 2)
        self.assertIn("profile-9.json", proc.stderr)


if __name__ == "__main__":
    unittest.main()
