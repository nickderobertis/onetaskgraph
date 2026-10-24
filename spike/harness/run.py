"""One measurement of one named scenario: `run.py <scenario> [--build stock|span]`.

Scenarios (see `spike/README.md`):

- `tb-first`, `tb-changed`, `tb-unchanged` — a `project copy` and then a `document copy` of
  the generated plan from a `local-md` folder onto onetaskgraph's test board, as
  ai-orchestrator's `copy-plan` sends them: the first creating every item, the second after
  every body changed, the third with nothing changed. Each build copies its own plan
  (`bcs-stock`, `bcs-span`), so the two builds see the same board but for each other's items.
- `host-copy`, `host-approve` — ai-orchestrator's own `just copy-plan` and `just
  approve-design`, in a scratch clone of it, its `plans` source pointed at the test board.
- `plans-walk` — one `project list --source plans --json` of ai-orchestrator's real `plans`
  board, the only request this spike sends it.
- `probes` — what GitHub does with `createIssue`'s `projectV2Ids` and `parentIssueId` and
  with one request of several aliased mutations, every issue deleted afterwards.
- `cleanup` — delete every test-board issue the generated plans were copied to.

Every request to GitHub goes through the recording proxy. Raw logs (whole bodies) stay under
the host cache directory; `sanitize.py` writes the committed form under `spike/logs/`.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
sys.path.insert(0, str(HERE))

import quota
import sanitize
import workload

CACHE = Path(os.environ.get("XDG_CACHE_HOME") or Path.home() / ".cache") / (
    "board-copy-speed-2026-10-18"
)
CLONE = Path(os.environ.get("SPIKE_HOST_CLONE") or CACHE / "ai-orchestrator")
SPAN_BINARY = Path(
    os.environ.get("SPIKE_SPAN_BINARY")
    or CACHE / "span-build/target/release/onetaskgraph"
)
STOCK_BINARY = CLONE / ".venv/bin/onetaskgraph"
LOGS = REPO / "spike/logs"
QUOTA_LOG = CACHE / "quota.jsonl"
PYTHON = shutil.which("python3", path="/usr/bin:/bin") or sys.executable

# The test board, as `crates/onetaskgraph-github-live/src/lib.rs` names it.
TEST_OWNER = "nickderobertis"
TEST_NUMBER = 1
TEST_REPOSITORY = "nickderobertis/onetaskgraph-live-scratch"

TB = ("tb-first", "tb-changed", "tb-unchanged")
SCENARIOS = (
    *TB,
    "host-copy",
    "host-approve",
    "plans-walk",
    "plans-board-walk",
    "tb-list-pages",
    "probes",
    "probe-body-limit",
    "cleanup",
)
UNREACHABLE = "http://127.0.0.1:9/graphql"


def token() -> str:
    value = os.environ.get("GH_PROJECTS_TOKEN", "").strip()
    if not value:
        sys.exit(
            "run.py: GH_PROJECTS_TOKEN is not set; export the test board's credential"
        )
    return value


def binary(build: str) -> Path:
    path = SPAN_BINARY if build == "span" else STOCK_BINARY
    if not path.exists():
        hint = (
            "spike/span-build/build.sh"
            if build == "span"
            else "spike/copy-timing.sh setup"
        )
        sys.exit(f"run.py: no {build} binary at {path}; run {hint}")
    version = subprocess.run(
        [path, "--version"], capture_output=True, text=True, check=False
    ).stdout
    if version.strip() != "onetaskgraph 0.7.1":
        sys.exit(f"run.py: {path} is {version.strip()!r}, not onetaskgraph 0.7.1")
    return path


def clean_env(**extra: str) -> dict[str, str]:
    """The caller's environment without any `ONETASKGRAPH_` layer of its own."""
    env = {k: v for k, v in os.environ.items() if not k.startswith("ONETASKGRAPH_")}
    for key in ("SPIKE_SPAN_LOG", "SPIKE_HOST_AUDIT", "HTTPS_PROXY", "https_proxy"):
        env.pop(key, None)
    env.update(extra)
    return env


def board_ids() -> list[str]:
    """The test board's project id and its scratch repository's id: the only ones a run's
    mutations may name. Read once, directly, and kept with the spike's state."""
    path = CACHE / "state" / "test-board-ids.json"
    if path.exists():
        return json.loads(path.read_text())
    owner, name = TEST_REPOSITORY.split("/")
    request = urllib.request.Request(
        "https://api.github.com/graphql",
        data=json.dumps(
            {
                "query": "query SpikeIds($o: String!, $n: String!, $p: Int!) { repository(owner: "
                "$o, name: $n) { id } user(login: $o) { projectV2(number: $p) { id } } }",
                "variables": {"o": owner, "n": name, "p": TEST_NUMBER},
            }
        ).encode(),
        headers={
            "Authorization": f"Bearer {token()}",
            "Content-Type": "application/json",
        },
    )
    with urllib.request.urlopen(request, timeout=60) as response:
        data = json.load(response)["data"]
    ids = [data["repository"]["id"], data["user"]["projectV2"]["id"]]
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(ids))
    return ids


class Proxy:
    def __init__(
        self, run: Path, writes: str, allowed: list[str], refused: tuple[str, ...] = ()
    ) -> None:
        self.log = run / "proxy.jsonl"
        port_file = run / "proxy.port"
        argv = [PYTHON, str(HERE / "proxy.py"), "--log", str(self.log)]
        argv += ["--port-file", str(port_file), "--writes", writes]
        for allowed_id in allowed:
            argv += ["--allow", allowed_id]
        for op in refused:
            argv += ["--refuse-op", op]
        self.process = subprocess.Popen(argv)
        for _ in range(100):
            if port_file.exists() and port_file.read_text():
                break
            time.sleep(0.1)
        self.port = int(port_file.read_text())
        self.endpoint = f"http://127.0.0.1:{self.port}/graphql"

    def env(self) -> dict[str, str]:
        tunnel = f"http://127.0.0.1:{self.port}"
        return {
            "HTTPS_PROXY": tunnel,
            "https_proxy": tunnel,
            "NO_PROXY": "127.0.0.1,localhost",
            "no_proxy": "127.0.0.1,localhost",
        }

    def stop(self) -> None:
        self.process.terminate()
        self.process.wait(timeout=30)


def step(run: Path, name: str, argv: list, env: dict, cwd: Path) -> int:
    """Run one process, recording its wall time; its output is kept raw beside the logs."""
    start = time.time()
    done = subprocess.run(
        [str(a) for a in argv], env=env, cwd=cwd, capture_output=True, check=False
    )
    end = time.time()
    (run / f"{name}.stdout").write_bytes(done.stdout)
    (run / f"{name}.stderr").write_bytes(done.stderr)
    with (run / "steps.jsonl").open("a") as out:
        record = {
            "step": name,
            "argv": [str(a) for a in argv],
            "start": start,
            "end": end,
        }
        out.write(json.dumps(record | {"returncode": done.returncode}) + "\n")
    print(f"  {name}: exit {done.returncode} in {end - start:.1f}s", flush=True)
    if done.returncode not in (0, 4):
        sys.stderr.write(done.stderr.decode(errors="replace")[-3000:])
    return done.returncode


# --- the test-board scenarios -------------------------------------------------------------


def tb_state(build: str) -> Path:
    return CACHE / "state" / f"bcs-{build}"


def tb_config(state: Path, endpoint: str) -> None:
    """The scenario's configuration: the generated folder and the test board, the latter
    configured as ai-orchestrator configures `plans` (cache on), its endpoint the proxy."""
    config = {
        "default_sources": ["spike-md"],
        "sources": {
            "spike-md": {"plugin": "local-md", "config": {"root": str(state / "md")}},
            "testboard": {
                "plugin": "github-projects",
                "config": {
                    "owner": TEST_OWNER,
                    "project_number": TEST_NUMBER,
                    "repository": TEST_REPOSITORY,
                    "token_env": "GH_PROJECTS_TOKEN",
                    "endpoint": endpoint,
                },
                "cache": {"enabled": True, "dir": str(state / "store-cache")},
            },
        },
    }
    (state / "onetaskgraph.yaml").write_text(json.dumps(config, indent=1))


def local_env(state: Path) -> dict[str, str]:
    (state / "xdg").mkdir(exist_ok=True)
    return clean_env(XDG_CONFIG_HOME=str(state / "xdg"))


def loader(path: Path) -> Path:
    done = subprocess.run(
        [CLONE / ".venv/bin/onepipeline", "template", "resolve", "design-doc", "--json"]
        + ["--template-root", CLONE / "templates"],
        capture_output=True,
        cwd=CLONE,
        env=clean_env(),
        check=True,
    )
    path.write_bytes(done.stdout)
    return path


def render_document(
    state: Path, plan: str, source: str, root: Path, env: dict, cwd: Path
) -> None:
    """Create, or regenerate, the plan's design document in its folder through the store."""
    out = state / "out"
    stock = [STOCK_BINARY]
    common = ["--template-loader", loader(state / "loader.json")]
    common += ["--answers", out / "design-answers.json", "--no-interactive", "--json"]
    document = root / "documents" / f"{plan}-design.md"
    if document.exists():
        argv = stock + ["document", "render", f"{source}:{plan}-design", *common]
    else:
        argv = stock + ["document", "create", source, "--project", plan]
        argv += ["--id", f"{plan}-design", "--title", f"Design: {plan}", *common]
        argv += ["--asset", out / "plan-dag.png"]
    done = subprocess.run(
        [str(a) for a in argv], env=env, cwd=cwd, capture_output=True, check=False
    )
    if done.returncode:
        sys.exit(
            f"run.py: rendering the document failed: {done.stderr.decode()[-2000:]}"
        )
    try:
        sizes = workload.rendered_sizes(root, plan)
    except ValueError as refused:
        sys.exit(f"run.py: the generated workload is not the brief's: {refused}")
    (state / "rendered-sizes.json").write_text(json.dumps(sizes, indent=1))


def record_workload(run: Path, state: Path) -> None:
    """Keep the rendered body sizes this run copied (bytes, characters) beside its logs."""
    sizes = state / "rendered-sizes.json"
    if sizes.exists():
        (run / "workload.json").write_text(sizes.read_text())


def tb_setup(build: str, scenario: str) -> tuple[Path, str]:
    state = tb_state(build)
    plan = state.name
    env = local_env(state) if state.exists() else None
    if scenario == "tb-first":
        if (state / "md").exists():
            sys.exit(
                f"run.py: {state} already holds a copied plan; run `cleanup` first"
            )
        state.mkdir(parents=True, exist_ok=True)
        env = local_env(state)
        tb_config(state, UNREACHABLE)
        workload.main_args(state / "md", plan, state / "out", variant=0, rewrite=False)
        render_document(state, plan, "spike-md", state / "md", env, state)
        (state / "variant").write_text("0")
    elif scenario == "tb-changed":
        variant = int((state / "variant").read_text()) + 1
        workload.main_args(
            state / "md", plan, state / "out", variant=variant, rewrite=True
        )
        render_document(state, plan, "spike-md", state / "md", env, state)
        (state / "variant").write_text(str(variant))
    return state, plan


def run_tb(run: Path, scenario: str, build: str, proxy: Proxy) -> None:
    state, plan = tb_setup(build, scenario)
    record_workload(run, state)
    tb_config(state, proxy.endpoint)
    env = local_env(state) | proxy.env() | {"GH_PROJECTS_TOKEN": token()}
    if build == "span":
        env["SPIKE_SPAN_LOG"] = str(run / "spans.jsonl")
    exe = binary(build)
    step(
        run,
        "project-copy",
        [exe, "project", "copy", f"spike-md:{plan}"] + ["--to", "testboard", "--json"],
        env,
        state,
    )
    step(
        run,
        "document-copy",
        [exe, "document", "copy", f"spike-md:{plan}-design"]
        + ["--to", "testboard", "--json"],
        env,
        state,
    )


# --- ai-orchestrator's recipes ------------------------------------------------------------

HOST_PLAN = "bcs-host"


def host_env(
    proxy: Proxy | None, run: Path | None, walk: bool = False
) -> dict[str, str]:
    """The scratch clone's environment: its `plans` source on the test board (or, for the
    walk, left on the real board), behind the proxy; `followups` pointed nowhere."""
    state = CACHE / "state" / HOST_PLAN
    env = clean_env(
        ONETASKGRAPH_SOURCES__AUTHORING__CONFIG__ROOT=str(state / "md"),
        ONETASKGRAPH_SOURCES__PLANS__CONFIG__ENDPOINT=proxy.endpoint
        if proxy
        else UNREACHABLE,
        ONETASKGRAPH_SOURCES__PLANS__CACHE__DIR=str(
            state / ("walk-cache" if walk else "cache")
        ),
        ONETASKGRAPH_SOURCES__FOLLOWUPS__CONFIG__ENDPOINT=UNREACHABLE,
        # `plans` routes `petsinc` tasks to the `hellopatient` Linear source, which a copy
        # builds although no task of this plan routes there. The scratch process holds no
        # Linear key, so it is built with a placeholder and an endpoint that answers nothing:
        # whatever a copy asked of Linear would fail the copy rather than reach it.
        ONETASKGRAPH_SOURCES__HELLOPATIENT__CONFIG__API_KEY_ENV="SPIKE_NO_LINEAR_KEY",
        ONETASKGRAPH_SOURCES__HELLOPATIENT__CONFIG__ENDPOINT=UNREACHABLE,
        SPIKE_NO_LINEAR_KEY="placeholder",
        GH_PROJECTS_TOKEN=token(),
    )
    if not walk:
        env["ONETASKGRAPH_SOURCES__PLANS__CONFIG__OWNER"] = TEST_OWNER
        env["ONETASKGRAPH_SOURCES__PLANS__CONFIG__PROJECT_NUMBER"] = str(TEST_NUMBER)
        env["ONETASKGRAPH_SOURCES__PLANS__CONFIG__REPOSITORY"] = TEST_REPOSITORY
    if proxy:
        env |= proxy.env()
    if run:
        env["SPIKE_HOST_AUDIT"] = str(run / "host.jsonl")
        env["PYTHONPATH"] = str(HERE / "hostaudit")
    return env


def host_setup() -> None:
    state = CACHE / "state" / HOST_PLAN
    if (state / "md").exists():
        return
    state.mkdir(parents=True, exist_ok=True)
    workload.main_args(
        state / "md", HOST_PLAN, state / "out", variant=0, rewrite=False, bare_last=True
    )
    env = host_env(None, None)
    render_document(state, HOST_PLAN, "authoring", state / "md", env, CLONE)
    stamp = ["uv", "run", "python", HERE / "stamp_reviews.py", f"authoring:{HOST_PLAN}"]
    done = subprocess.run(
        [str(a) for a in stamp], env=env, cwd=CLONE, capture_output=True, check=False
    )
    if done.returncode:
        sys.exit(
            f"run.py: stamping review records failed: {done.stderr.decode()[-3000:]}"
        )


def board_project(state: Path, plan: str, source: str) -> str:
    """The board id a copy recorded on the local project, as `<source>:<id>`."""
    text = (state / "md" / "projects" / f"{plan}.md").read_text()
    match = re.search(r'"onetaskgraph\.copies":\s*(\{.*\})', text)
    if not match:
        sys.exit(f"run.py: {plan} records no copy; run the copy scenario first")
    return json.loads(match.group(1))[source]


def run_host(run: Path, scenario: str, build: str, proxy: Proxy) -> None:
    if build != "stock":
        sys.exit(
            "run.py: the host recipes run the onetaskgraph-cli wheel the clone locks"
        )
    host_setup()
    record_workload(run, CACHE / "state" / HOST_PLAN)
    env = host_env(proxy, run)
    if scenario == "host-copy":
        step(
            run,
            "copy-plan",
            ["just", "copy-plan", f"authoring:{HOST_PLAN}"],
            env,
            CLONE,
        )
    else:
        target = board_project(CACHE / "state" / HOST_PLAN, HOST_PLAN, "plans")
        step(run, "approve-design", ["just", "approve-design", target], env, CLONE)


def run_walk(run: Path, build: str, proxy: Proxy, verb: str = "project") -> None:
    env = host_env(proxy, None, walk=True)
    argv = [binary(build), verb, "list", "--source", "plans", "--json"]
    step(run, f"{verb}-list", argv, env, CLONE)


def run_list_pages(run: Path, build: str, proxy: Proxy) -> None:
    """`project list` of the test board two projects a page, following `next` to the end, as
    ai-orchestrator's `every_page` follows it: does each continuation walk the board again?"""
    state = tb_state("stock") if tb_state("stock").exists() else tb_state("span")
    tb_config(state, proxy.endpoint)
    env = local_env(state) | proxy.env() | {"GH_PROJECTS_TOKEN": token()}
    exe = binary(build)
    page: str | None = None
    for number in range(1, 11):
        argv = [
            exe,
            "project",
            "list",
            "--source",
            "testboard",
            "--limit",
            "2",
            "--json",
        ]
        step(
            run, f"page-{number}", argv + (["--page", page] if page else []), env, state
        )
        page = json.loads((run / f"page-{number}.stdout").read_text()).get("next")
        if not page:
            break


def header_floor(scenario: str) -> None:
    """Pause for the reset while the last GraphQL answer any run recorded, in the current
    window, left the bucket under the floor: the headers are what GitHub enforces, and the
    readings the gate takes have answered thousands of points too generously."""
    logs = sorted(
        (CACHE / "runs").glob("*/proxy.jsonl"), key=lambda p: p.stat().st_mtime
    )
    for log in reversed(logs[-3:]):
        records = [
            json.loads(line) for line in log.read_text().splitlines() if line.strip()
        ]
        graphql = [
            r for r in records if r["limits"].get("x-ratelimit-resource") == "graphql"
        ]
        if not graphql:
            continue
        last = max(graphql, key=lambda r: r["end"])["limits"]
        reset = int(last["x-ratelimit-reset"])
        share = int(last["x-ratelimit-remaining"]) / int(last["x-ratelimit-limit"])
        if share < quota.FLOOR and reset > time.time():
            wait = reset - time.time() + 5
            with QUOTA_LOG.open("a") as out:
                pause = {"event": "pause", "scenario": scenario, "at": time.time()}
                pause |= {
                    "seconds": wait,
                    "buckets": ["graphql (last recorded headers)"],
                }
                out.write(json.dumps(pause, sort_keys=True) + "\n")
            print(
                f"{scenario}: last headers {share:.0%}; waiting {wait:.0f}s", flush=True
            )
            time.sleep(wait)
        return


# --- driver -------------------------------------------------------------------------------

DRAWS = {
    "probes": ("graphql",),
    "probe-body-limit": ("graphql",),
    "plans-walk": ("graphql",),
    "plans-board-walk": ("graphql",),
}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("scenario", choices=SCENARIOS)
    parser.add_argument("--build", choices=("stock", "span"), default="stock")
    parser.add_argument("--no-sanitize", action="store_true")
    parser.add_argument(
        "--plan",
        help="cleanup: only this generated plan (bcs-stock, bcs-span, bcs-host)",
    )
    args = parser.parse_args()
    stamp = time.strftime("%Y%m%dT%H%M%S", time.gmtime())
    label = args.scenario
    host_project = CACHE / "state" / HOST_PLAN / "md" / "projects" / f"{HOST_PLAN}.md"
    if (
        label == "host-copy"
        and host_project.exists()
        and "onetaskgraph.copies" in host_project.read_text()
    ):
        label = "host-recopy"  # the plan is already on the board: an unchanged re-copy
    name = f"{label}-{args.build}"
    run = CACHE / "runs" / f"{name}-{stamp}"
    run.mkdir(parents=True)
    print(f"{name}: raw logs in {run}", flush=True)
    header_floor(name)
    reading = quota.gate(
        token(), QUOTA_LOG, name, DRAWS.get(args.scenario, quota.BUCKETS)
    )
    (run / "quota-before.json").write_text(json.dumps(reading))
    walk = args.scenario in ("plans-walk", "plans-board-walk")
    # The BOARD walk of `plans` is `task list`'s first half; its search half is refused, so
    # exactly the BOARD pages the manager allowed leave, and the command then fails.
    refused = ("SEARCH_ISSUES",) if args.scenario == "plans-board-walk" else ()
    proxy = Proxy(
        run, "none" if walk else "allowed", [] if walk else board_ids(), refused
    )
    try:
        if args.scenario in TB:
            run_tb(run, args.scenario, args.build, proxy)
        elif args.scenario in ("host-copy", "host-approve"):
            run_host(run, args.scenario, args.build, proxy)
        elif args.scenario == "plans-walk":
            run_walk(run, args.build, proxy)
        elif args.scenario == "plans-board-walk":
            run_walk(run, args.build, proxy, verb="task")
        elif args.scenario == "tb-list-pages":
            run_list_pages(run, args.build, proxy)
        elif args.scenario == "probe-body-limit":
            import probes

            probes.body_limit(run, proxy.endpoint, token(), TEST_OWNER, TEST_REPOSITORY)
        elif args.scenario == "probes":
            import probes

            probes.run(
                run, proxy.endpoint, token(), TEST_OWNER, TEST_NUMBER, TEST_REPOSITORY
            )
        else:
            import cleanup

            cleanup.run(run, proxy.endpoint, token(), CACHE / "state", args.plan)
    finally:
        proxy.stop()
        after = quota.read(token())
        (run / "quota-after.json").write_text(
            json.dumps({"at": time.time(), "buckets": after})
        )
    if not args.no_sanitize:
        out = sanitize.write(run, LOGS / f"{name}-{stamp}")
        print(f"{name}: sanitized logs in {out.relative_to(REPO)}", flush=True)


if __name__ == "__main__":
    main()
