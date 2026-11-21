"""taskgraphs.py through its entry point, over a real git repository, with a stand-in `nx` at the Nx
process boundary that records where and how it was asked for the graph."""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
import textwrap
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent


def git(root: Path, *args: str) -> str:
    done = subprocess.run(
        ["git", "-C", str(root), "-c", "user.name=t", "-c", "user.email=t@t", *args],
        capture_output=True,
        text=True,
        check=True,
    )
    return done.stdout.strip()


class TaskGraphsTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="taskgraphs-"))
        self.addCleanup(shutil.rmtree, self.root, True)
        self.repo = self.root / "repo"
        (self.repo / "crates/core").mkdir(parents=True)
        (self.repo / "crates/core/project.json").write_text('{"name": "core"}\n')
        git(self.repo, "init", "-q")
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-qm", "the PR's head")
        self.sha = git(self.repo, "rev-parse", "HEAD")
        nx = self.repo / "node_modules/.bin/nx"
        nx.parent.mkdir(parents=True)
        nx.write_text(
            textwrap.dedent(f"""\
            #!{Path(sys.executable).absolute()}
            import json, os, sys
            with open({str(self.root / "asked.jsonl")!r}, "a") as log:
                log.write(json.dumps({{"cwd": os.getcwd(), "args": sys.argv[1:],
                                      "core": os.path.exists("crates/core/project.json")}}) + "\\n")
            print(json.dumps({{"graph": {{}}, "tasks": {{"tasks": {{}},
                "dependencies": {{"e2e:test": ["core:build"], "core:build": []}}}}}}))
            """)
        )
        nx.chmod(0o755)
        (self.repo / ".gitignore").write_text("node_modules\n")
        self.history = self.root / "history.json"
        job = {
            "job_id": 1,
            "source": "executions",
            "wall_s": 60,
            "conclusion": "success",
            "tasks": {"e2e:test": {"result": "executed", "seconds": 30, "share_s": 30}},
        }
        record = {
            "run": 7,
            "head_sha": self.sha,
            "created_at": "2026-11-12T04:00:00Z",
            "conclusion": "success",
            "runs_of_pr": 1,
            "jobs": {"test-macos": job},
        }
        self.history.write_text(json.dumps({"4000": record}))

    def run_tool(self, *args: str) -> subprocess.CompletedProcess:
        return subprocess.run(
            [str(Path(sys.executable).absolute()), str(HERE / "taskgraphs.py"), *args],
            capture_output=True,
            text=True,
            check=False,
        )

    def test_each_prs_graph_is_read_at_its_own_commit_and_its_worktree_removed(self):
        out = self.root / "graphs"
        proc = self.run_tool("history", str(self.history), "--out", str(out), "--repo", str(self.repo))
        self.assertEqual(proc.returncode, 0, proc.stderr)
        written = json.loads((out / "pr-4000.json").read_text())
        self.assertEqual(written["sha"], self.sha)
        self.assertEqual(written["targets"], ["test"])
        self.assertEqual(written["dependencies"], {"e2e:test": ["core:build"], "core:build": []})
        asked = [json.loads(line) for line in (self.root / "asked.jsonl").read_text().splitlines()]
        self.assertEqual(asked[0]["args"], ["run-many", "--all", "-t", "test", "--graph=stdout"])
        self.assertTrue(asked[0]["core"])  # run inside the PR's own tree
        self.assertNotEqual(Path(asked[0]["cwd"]).resolve(), self.repo.resolve())
        self.assertEqual(len(git(self.repo, "worktree", "list").splitlines()), 1)

    def test_a_commit_this_repository_does_not_hold_is_named_and_writes_nothing(self):
        record = json.loads(self.history.read_text())
        record["4000"]["head_sha"] = "0" * 40
        self.history.write_text(json.dumps(record))
        out = self.root / "graphs"
        proc = self.run_tool("history", str(self.history), "--out", str(out), "--repo", str(self.repo))
        self.assertEqual(proc.returncode, 4)
        self.assertIn("pr-4000: " + "0" * 40, proc.stderr)
        self.assertFalse((out / "pr-4000.json").exists())


if __name__ == "__main__":
    unittest.main()
