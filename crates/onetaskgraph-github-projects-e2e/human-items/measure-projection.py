#!/usr/bin/env python3
"""One offline measurement; raw requests remain beside the summary. No live writes."""
from collections import defaultdict
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[3]
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ)
    env.update(PROJECTION_SPIKE_OUTPUT=str(output / "github.json"),
               LINEAR_PROJECTION_SPIKE_OUTPUT=str(output / "linear.json"),
               GITHUB_ASSET_TELEMETRY_DIR=str(output / "approval-telemetry"))
    started = time.monotonic()
    commands = [
        ["cargo", "test", "-p", "onetaskgraph", "--no-run", "--locked"],
        ["cargo", "test", "-p", "onetaskgraph-github-projects-e2e", "--test", "e2e",
         "--locked", "projection_spike::measure_projection_spike", "--", "--exact", "--nocapture"],
        ["cargo", "test", "-p", "onetaskgraph-linear-e2e", "--test", "e2e",
         "--locked", "projection_spike::measure_linear_projection_spike", "--", "--exact", "--nocapture"],
    ]
    for index, command in enumerate(commands):
        print("Running:", " ".join(command), flush=True)
        with (output / f"command-{index}.log").open("w") as log:
            subprocess.run(command, cwd=root, env=env, stdout=log,
                           stderr=subprocess.STDOUT, check=True)
    github = json.loads((output / "github.json").read_text())
    linear = json.loads((output / "linear.json").read_text())
    measured_files = [
        root / "crates/onetaskgraph-github-projects-e2e/tests/e2e/projection_spike.rs",
        root / "crates/onetaskgraph-github-projects-e2e/tests/e2e/approval.rs",
        root / "crates/onetaskgraph-github-projects-e2e/tests/e2e/asset_board.rs",
        root / "crates/onetaskgraph-linear-e2e/tests/e2e/projection_spike.rs",
        root / "crates/onetaskgraph-e2e-support/src/fixtures.rs",
        Path(__file__).resolve(),
    ]
    breakdown = {}
    for mode in ("existing", "prototype"):
        groups = defaultdict(lambda: {"requests": 0, "points": 0, "mutation_fields": 0, "secondary_points": 0})
        for manager in github[mode]["managers"]:
            for row in manager["rows"]:
                for request in row["raw"]:
                    group = groups[row["phase"] + ": " + request["operation"]]
                    group["requests"] += 1
                    for name in ("points", "mutation_fields", "secondary_points"):
                        group[name] += request[name]
        breakdown[mode] = dict(sorted(groups.items()))
    first_rows = github["prototype"]["managers"][0]["rows"]
    single = []
    for row in first_rows:
        if row["phase"] in ("shared_project_preflight", "create", "resolve"):
            single.append(row)
        if row["phase"] == "resolve":
            break
    first_task_increment = {name: sum(row[name] for row in single)
                            for name in ("requests", "points", "mutation_fields", "secondary_points")}
    result = {
        "schema_version": 1,
        "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=root, text=True).strip(),
        "measured_file_sha256": {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
                                 for path in measured_files},
        "workload": {"tasks": 100, "managers": 10, "board_items": 2000},
        "elapsed_seconds": time.monotonic() - started,
        "github": {key: {name: github[key][name] for name in (
            "normal_requests", "normal_points", "points_per_item", "requests_including_retries",
            "points_including_retries", "secondary_points")} for key in ("existing", "prototype")},
        "operation_breakdown": breakdown,
        "measured_cold_one_task_increment": first_task_increment,
        "approval": github["approval"],
        "linear_requests": linear["requests"],
        "github_consumed_requests": sum(github[mode]["requests_including_retries"] for mode in ("existing", "prototype")) + sum(flow["consumed_requests"] for flow in github["approval"]),
        "github_consumed_modeled_points": sum(github[mode]["points_including_retries"] for mode in ("existing", "prototype")) + sum(flow["consumed_points"] for flow in github["approval"]),
        "live_requests": 0,
        "live_points": 0,
        "point_authority": "GitHub plugin worst_case_point_cost per emitted document; offline model, not live quota",
    }
    (output / "summary.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
