#!/usr/bin/env python3
"""Read Nx's own task graph at the commits a replay needs.

    python3 spike/ci-profile/taskgraphs.py history HISTORY --out DIR
    python3 spike/ci-profile/taskgraphs.py runs SUMMARIES RUN... --out DIR

For each commit — the head each PR's cited run tested (`history`), or each demonstration run's
head (`runs`, from `profile-<run>.json` and `run.json` in SUMMARIES' read cache) — it checks the
commit out into a temporary git worktree, borrows this checkout's `node_modules`, and asks Nx for
`run-many --all -t <every target that run executed> --graph=stdout`. It writes `DIR/<key>.json`:
the commit, the targets, and each task's dependencies, which `critical_path.py` follows. The
commit must already be in this repository (fetch a PR's `refs/pull/<n>/head` first); one that is
not is named, and nothing is written for it.

Exit status: 0 every graph written; 2 a usage error; 4 a commit was missing or Nx refused one
(named on standard error, the others still written).
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

# E402: the harness's own modules are found through the sys.path entry above.
from models import history, obj  # noqa: E402

REPO = Path(__file__).resolve().parents[2]
USAGE_EXIT, MISSING_EXIT = 2, 4


def graph_at(sha: str, targets: list[str], repo: Path = REPO) -> dict[str, list[str]]:
    """Nx's task dependencies for `targets` over every project, at commit `sha`."""
    with tempfile.TemporaryDirectory(prefix="taskgraph-") as scratch:
        tree = Path(scratch) / "tree"
        add = subprocess.run(
            ["git", "-C", str(repo), "worktree", "add", "--detach", "-q", str(tree), sha],
            capture_output=True,
            text=True,
            check=False,
        )
        if add.returncode != 0:
            raise LookupError(f"{sha}: {add.stderr.strip()}")
        try:
            (tree / "node_modules").symlink_to(repo / "node_modules", target_is_directory=True)
            nx = subprocess.run(
                [str(tree / "node_modules/.bin/nx"), "run-many", "--all", "-t", ",".join(targets), "--graph=stdout"],
                cwd=tree,
                capture_output=True,
                text=True,
                env={**os.environ, "NX_DAEMON": "false", "NX_NO_CLOUD": "true"},
                check=False,
            )
            if nx.returncode != 0:
                raise LookupError(f"{sha}: nx refused the graph: {nx.stderr.strip()[-400:]}")
            tasks = obj(obj(json.loads(nx.stdout), "nx graph").get("tasks"), "nx graph tasks")
            return {str(k): [str(d) for d in v] for k, v in obj(tasks.get("dependencies"), "dependencies").items()}
        finally:
            subprocess.run(
                ["git", "-C", str(repo), "worktree", "remove", "--force", str(tree)], capture_output=True, check=False
            )


def write(out: Path, key: str, sha: str, targets: list[str], repo: Path = REPO) -> str | None:
    """Write `DIR/<key>.json`, or say why it could not be read."""
    try:
        deps = graph_at(sha, targets, repo)
    except LookupError as error:
        return f"{key}: {error}"
    (out / f"{key}.json").write_text(json.dumps({"sha": sha, "targets": targets, "dependencies": deps}, indent=1))
    return None


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(
        prog="taskgraphs.py", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    sub = ap.add_subparsers(dest="cmd", required=True)
    h = sub.add_parser("history")
    h.add_argument("history", type=Path)
    h.add_argument("--out", type=Path, required=True)
    h.add_argument("--repo", type=Path, default=REPO, help="the checkout whose node_modules and commits are read")
    r = sub.add_parser("runs")
    r.add_argument("summaries", type=Path)
    r.add_argument("runs", nargs="+")
    r.add_argument("--out", type=Path, required=True)
    r.add_argument("--repo", type=Path, default=REPO, help="the checkout whose node_modules and commits are read")
    args = ap.parse_args(argv)
    args.out.mkdir(parents=True, exist_ok=True)
    missing: list[str | None] = []
    try:
        if args.cmd == "history":
            for pr, first in history(json.loads(args.history.read_text())).items():
                targets = sorted({str(t).split(":")[1] for j in first.jobs.values() for t in j.tasks})
                missing.append(write(args.out, f"pr-{pr}", first.head_sha, targets, args.repo.resolve()))
        else:
            for run in args.runs:
                summary = obj(json.loads((args.summaries / f"profile-{run}.json").read_text()), f"profile-{run}")
                sha = str(summary.get("head_sha") or "")
                jobs = obj(summary.get("jobs"), f"profile-{run} jobs").values()
                targets = sorted({str(t["task"]).split(":")[1] for j in jobs for t in j["tasks"]})
                missing.append(write(args.out, f"run-{run}", sha, targets, args.repo.resolve()))
    except (OSError, ValueError, KeyError) as error:
        print(f"taskgraphs: {type(error).__name__}: {error}", file=sys.stderr)
        return USAGE_EXIT
    refused = [line for line in missing if line]
    for line in refused:
        print(f"taskgraphs: {line}", file=sys.stderr)
    return MISSING_EXIT if refused else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
