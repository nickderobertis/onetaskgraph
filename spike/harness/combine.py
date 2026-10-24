"""The before figures at the `plans` board's real size, from the committed logs.

Each test-board scenario's walk pages (BOARD and SEARCH_ISSUES pages, one each per walk on
a board of about 100 items) are replaced by the pages the same walk takes on `plans`, at the
latency and price measured there: 20 BOARD pages (its 1,986 items, `plans-board-walk`) and
10 SEARCH_ISSUES pages (GitHub stops a search at 1,000 results, `plans-walk`). Approval's
listing is `project list --source plans` once per 50 projects ai-orchestrator's `every_page`
reads, and each continuation walks the search again (`tb-list-pages`); the 1,000 issues the
search reaches hold `PROJECTS` projects, so 3 processes. Everything else is the test board's
measured time, unscaled.

`python3 combine.py` writes `spike/logs/combined.json` and prints the arithmetic.
"""

from __future__ import annotations

import json
import math
import statistics
from pathlib import Path

LOGS = Path(__file__).resolve().parent.parent / "logs"
PROJECTS = (
    106  # projects among the 1,000 issues the `plans` search reached (cached pages)
)
PAGE = 50  # the store's default page size, which `every_page` follows


def lines(path: Path) -> list[dict]:
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def latest(prefix: str) -> Path:
    return max(p for p in LOGS.iterdir() if p.is_dir() and p.name.startswith(prefix))


def page_mean(run: Path, op: str) -> tuple[float, int]:
    pages = [
        r for r in lines(run / "requests.jsonl") if r["op"] == op and r["status"] == 200
    ]
    return statistics.fmean(r["ms"] for r in pages) / 1000, len(pages)


def main() -> None:
    prices = json.loads((LOGS / "pricing.json").read_text())
    summary = json.loads((LOGS / "summary.json").read_text())["runs"]
    t_board, n_board = page_mean(latest("plans-board-walk-"), "BOARD")
    t_search, n_search = page_mean(latest("plans-walk-"), "SEARCH_ISSUES")
    p_board, p_search = prices["BOARD"]["points"], prices["SEARCH_ISSUES"]["points"]
    listings = math.ceil(PROJECTS / PAGE)
    out = {
        "plans": {
            "board_page_seconds": round(t_board, 3),
            "board_pages_per_walk": n_board,
            "search_page_seconds": round(t_search, 3),
            "search_pages_per_walk": n_search,
            "approval_listing_processes": listings,
        },
        "scenarios": {},
    }
    print(
        f"plans: BOARD {n_board} pages x {t_board:.2f} s; SEARCH {n_search} pages x {t_search:.2f} s"
    )
    for prefix in (
        "tb-first-span-",
        "tb-changed-span-",
        "tb-unchanged-span-",
        "host-copy-stock-",
        "host-recopy-stock-",
        "host-approve-stock-",
    ):
        name = latest(prefix).name
        run = summary[name]
        requests = lines(LOGS / name / "requests.jsonl")
        boards = [r for r in requests if r["op"] == "BOARD"]
        searches = [r for r in requests if r["op"] == "SEARCH_ISSUES"]
        walk_seconds = sum(r["ms"] for r in boards + searches) / 1000
        scale = listings if prefix.startswith("host-approve") else 1
        board_pages = len(boards) * n_board * scale
        search_pages = len(searches) * n_search * scale
        seconds = (
            run["wall"] - walk_seconds + board_pages * t_board + search_pages * t_search
        )
        points = (
            run["points"]
            - len(boards) * p_board
            - len(searches) * p_search
            + board_pages * p_board
            + search_pages * p_search
        )
        requests_at_scale = (
            run["requests"] - len(boards) - len(searches) + board_pages + search_pages
        )
        arithmetic = (
            f"{run['wall']:.1f} s measured - {walk_seconds:.1f} s of {len(boards)} BOARD + "
            f"{len(searches)} SEARCH test-board pages + {board_pages} x {t_board:.2f} s + "
            f"{search_pages} x {t_search:.2f} s = {seconds:.0f} s"
        )
        out["scenarios"][name] = {
            "seconds": round(seconds, 1),
            "walk_share": round(
                (board_pages * t_board + search_pages * t_search) / seconds, 3
            ),
            "points": points,
            "requests": requests_at_scale,
            "board_pages": board_pages,
            "search_pages": search_pages,
            "arithmetic": arithmetic,
        }
        print(f"{name}: {arithmetic}; {points} points; {requests_at_scale} requests")
    (LOGS / "combined.json").write_text(
        json.dumps(out, indent=1, sort_keys=True) + "\n"
    )


if __name__ == "__main__":
    main()
