# llmlint: ignore-file[structural_pattern_matching] This repository's Python target is 3.9 (requires-python in pyproject.toml), where a match statement does not parse.
# llmlint: ignore-file[new_code_lands_in_a_project] This is the measurement record beside the captures it produced: run by hand against GitHub through measure.sh, never by a target. docs/measurements is no Nx project (nor is coalesced-members-dry-run/dry_run.py beside it), and making it one would put a live-GitHub harness inside affected selection.
# llmlint: ignore-file[async_typed_clients_at_boundaries] These sequential urllib and http.client calls are the clients that took the committed captures (the 2026-10-18 timing harness, adapted only where the adopted releases required); one request at a time is deliberate so a row's timing is not perturbed, and rewriting them would make this no longer the code that measured.
# llmlint: ignore-file[boundary_inputs_validated] Every input here is this harness's own raw log, its own state folder or the plan it generated moments earlier; the committed captures are validated where they are read, by analyze.py, and what leaves the raw logs is held by the sanitizer's allowlist and leak scan.
# llmlint: ignore-file[changed_behavior_has_e2e] Its proof is the live run itself, whose captures are committed and reconciled by test_analyze.py; the GitHub-facing parts can be driven only against GitHub's test board, which measure.sh does, and the sanitizer produces every capture those tests read.
# llmlint: ignore-file[contracts_have_one_source_or_a_drift_gate] What this mirrors (the plugin's documents, the host's configuration, the scratch identities) is pinned to the releases measured, onetaskgraph v0.12.7 and ai-orchestrator 668dc496, each stated in the report and in every row's versions.json; the capture schema it shares with analyze.py is held by test_analyze.py, which builds every capture through sanitize.write.
# llmlint: ignore-file[modern_domain_modeling] These records are JSON lines passed by allowlist from the proxy's raw log to the sanitized captures; the typed reading of them is analyze.py, and remodelling the retained harness would make it no longer the code that took the captures.
"""One measurement of one named scenario: `run.py <scenario>`.

The board-copy-targets measurement (`docs/measurements/board-copy-targets-2026-10-26.md`). It is
the board-copy-speed measurement's harness (`board-copy-speed-2026-10-18.md`), kept as that run
left it but for what the releases this host adopted, and the private source, require.

Scenarios:

- `bare-first`, `bare-changed1`, `bare-unchanged` — a `project copy` and then a `document copy`
  of the generated plan, run by the store's own binary, from a `local-md` folder declared
  `visibility: private` onto onetaskgraph's test board, configured as ai-orchestrator configures
  `plans` (declared public, cache on, its `status_mapping`) and with the host's `write_policy`:
  the first creating every item, the second after one task's body changed, the third with
  nothing changed.
- `host-first`, `host-changed1`, `host-unchanged` — ai-orchestrator's own `just copy-plan`, in a
  scratch clone of it, its `authoring` source (declared private in its own `onetaskgraph.yaml`)
  rooted at a generated folder, its `plans` source pointed at the test board.
- `host-approve` — ai-orchestrator's `just approve-design` of the host plan's board project.
- `cleanup [--only bare|host]` — delete every test-board issue the generated plans were copied
  to (or only one side's).

Every request to GitHub goes through the recording proxy, which refuses any mutation naming a
repository or project other than the test board's. Raw logs (whole bodies) stay under the host
cache directory; `sanitize.py` writes the committed form beside the report.
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
MEASUREMENT = HERE.parent
REPO = MEASUREMENT.parent.parent.parent
sys.path.insert(0, str(HERE))

import quota
import sanitize
import workload

CACHE = Path(os.environ.get("XDG_CACHE_HOME") or Path.home() / ".cache") / (
    "board-copy-targets-2026-10-26"
)
CLONE = Path(os.environ.get("SPIKE_HOST_CLONE") or CACHE / "ai-orchestrator")
HOST_COMMIT = os.environ.get("SPIKE_HOST_COMMIT", "668dc496")
STOCK_BINARY = CLONE / ".venv/bin/onetaskgraph"
ONEVCS = CLONE / ".venv/bin/onevcs"
ONEPIPELINE = CLONE / "node_modules/.bin/onepipeline"
LOGS = Path(os.environ.get("SPIKE_LOGS") or MEASUREMENT / "captures")
RUNS = CACHE / os.environ.get("SPIKE_RUNS_DIR", "runs")
VERSION = os.environ.get("SPIKE_VERSION", "onetaskgraph 0.12.7")
QUOTA_LOG = CACHE / "quota.jsonl"
PYTHON = shutil.which("python3", path="/usr/bin:/bin") or sys.executable

# The test board, as `crates/onetaskgraph-github-live/src/lib.rs` names it.
TEST_OWNER = "nickderobertis"
TEST_NUMBER = 1
TEST_REPOSITORY = "nickderobertis/onetaskgraph-live-scratch"

# One generated plan for both the bare store and the host, so both copy the workload the brief
# states (its sizes are seeded by the plan's name); each lives in its own folder and its own
# copy links, so the two never share a board item.
PLAN = "bcs-span"
BARE = ("bare-first", "bare-changed1", "bare-unchanged")
HOST = ("host-first", "host-changed1", "host-unchanged", "host-approve")
SCENARIOS = (*BARE, *HOST, "cleanup")
UNREACHABLE = "http://127.0.0.1:9/graphql"
HOST_XDG = os.environ.get("XDG_CONFIG_HOME") or str(Path.home() / ".config")

# ai-orchestrator's `plans` source's `status_mapping` at the adopted commit, so the bare store's
# board is configured as the host's is.
PLANS_STATUS_MAPPING = {
    "unknown": "Needs attention",
    "queued": "Queued",
    "done": "Done",
    "cancelled": "Cancelled",
}


def token() -> str:
    value = os.environ.get("GH_PROJECTS_TOKEN", "").strip()
    if not value:
        sys.exit(
            "run.py: GH_PROJECTS_TOKEN is not set; export the test board's credential"
        )
    return value


def binary() -> Path:
    if not STOCK_BINARY.exists():
        sys.exit(
            f"run.py: no store binary at {STOCK_BINARY}; run `measure.sh provision`"
        )
    version = subprocess.run(
        [STOCK_BINARY, "--version"], capture_output=True, text=True, check=False
    ).stdout
    if version.strip() != VERSION:
        sys.exit(f"run.py: {STOCK_BINARY} is {version.strip()!r}, not {VERSION}")
    return STOCK_BINARY


def clean_env(**extra: str) -> dict[str, str]:
    """The caller's environment without any `ONETASKGRAPH_` layer of its own."""
    env = {k: v for k, v in os.environ.items() if not k.startswith("ONETASKGRAPH_")}
    for key in (
        "SPIKE_HOST_AUDIT",
        "HTTPS_PROXY",
        "https_proxy",
        "PYTHONPATH",
        "VIRTUAL_ENV",
    ):
        env.pop(key, None)
    env.update(extra)
    return env


def versions() -> dict[str, str]:
    """What this run measured: the store, the engine, the SDK, onevcs and the host commit."""

    def first_line(argv: list) -> str:
        done = subprocess.run(
            [str(a) for a in argv],
            capture_output=True,
            text=True,
            check=False,
            cwd=CLONE,
        )
        return (done.stdout.strip().splitlines() or ["unavailable"])[0]

    sdk = first_line(
        [
            CLONE / ".venv/bin/python",
            "-c",
            "import importlib.metadata as m; print(m.version('onetaskgraph-sdk'))",
        ]
    )
    return {
        "store": first_line([STOCK_BINARY, "--version"]),
        "engine": first_line([ONEPIPELINE, "--version"]),
        "sdk": f"onetaskgraph-sdk {sdk}",
        "onevcs": first_line([ONEVCS, "--version"]),
        "host_commit": first_line(["git", "rev-parse", "--short=8", "HEAD"]),
        "harness_commit": subprocess.run(
            ["git", "rev-parse", "--short=8", "HEAD"],
            capture_output=True,
            text=True,
            check=False,
            cwd=REPO,
        ).stdout.strip(),
    }


def cache_state(directory: Path) -> dict[str, int]:
    """How many files, and bytes, the store's cache directory holds: its state, not content."""
    files = (
        [p for p in directory.rglob("*") if p.is_file()] if directory.exists() else []
    )
    return {"files": len(files), "bytes": sum(p.stat().st_size for p in files)}


CONDITION = re.compile(
    r"sources\.[a-z-]+\.(plugin|visibility|cache\.enabled)|write_policy\.[a-z_]+"
)


def conditions(env: dict, cwd: Path) -> dict:
    """The configuration a run's store reads, as `config show` reports it, kept to what is
    declared — each source's plugin, visibility and cache switch, whether a `write_policy`
    command is set — and the layer each came from, so the user-layer condition is evidence."""
    done = subprocess.run(
        [STOCK_BINARY, "config", "show", "--json"],
        env={k: v for k, v in env.items() if k not in ("HTTPS_PROXY", "https_proxy")},
        cwd=cwd,
        capture_output=True,
        check=False,
    )
    if done.returncode:
        return {"error": "config show failed"}
    out: dict = {}
    for setting in json.loads(done.stdout)["settings"]:
        if not CONDITION.fullmatch(setting["key"]):
            continue
        value = setting["value"]
        if setting["key"].startswith("write_policy."):
            value = "set" if value else "unset"
        out[setting["key"]] = {"value": value, "layer": setting["origin"]["layer"]}
    return out


def board_ids() -> list[str]:
    """The test board's project id and its scratch repository's id: the only ones a run's
    mutations may name. Read once, directly, and kept with the measurement's state."""
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
    def __init__(self, run: Path, allowed: list[str]) -> None:
        self.log = run / "proxy.jsonl"
        port_file = run / "proxy.port"
        argv = [PYTHON, str(HERE / "proxy.py"), "--log", str(self.log)]
        argv += ["--port-file", str(port_file), "--writes", "allowed"]
        for allowed_id in allowed:
            argv += ["--allow", allowed_id]
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


FAILED_STEPS: list[str] = []


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
        FAILED_STEPS.append(name)
    return done.returncode


def screening(run: Path | None) -> dict[str, list[str]]:
    """The host's `write_policy`, both commands through the real onevcs behind the shim that
    logs each call and stands a registered public repository in for the scratch one."""
    shim = [PYTHON, "-I", str(HERE / "inspect_shim.py"), "--xdg", HOST_XDG, "--log"]
    shim += [str(run / "screen.jsonl") if run else "-", str(ONEVCS), "boundary"]
    return {
        "visibility_command": shim + ["inspect", "--input", "-"],
        "check_command": shim + ["check", "--destination", "public", "--input", "-"],
    }


BARE_STATE = CACHE / "state" / "bare"


def bare_config(endpoint: str, run: Path | None) -> None:
    """The bare store's configuration: the generated folder declared private, as the host's
    `authoring` is, and the test board configured as ai-orchestrator configures `plans`."""
    state = BARE_STATE
    config = {
        "write_policy": screening(run),
        "default_sources": ["spike-md"],
        "sources": {
            "spike-md": {
                "plugin": "local-md",
                "visibility": "private",
                "config": {"root": str(state / "md")},
            },
            "testboard": {
                "plugin": "github-projects",
                "visibility": "public",
                "config": {
                    "owner": TEST_OWNER,
                    "project_number": TEST_NUMBER,
                    "repository": TEST_REPOSITORY,
                    "token_env": "GH_PROJECTS_TOKEN",
                    "endpoint": endpoint,
                    "status_mapping": PLANS_STATUS_MAPPING,
                },
                "cache": {"enabled": True, "dir": str(state / "store-cache")},
            },
        },
    }
    (state / "onetaskgraph.yaml").write_text(json.dumps(config, indent=1))


def bare_env() -> dict[str, str]:
    (BARE_STATE / "xdg").mkdir(parents=True, exist_ok=True)
    return clean_env(XDG_CONFIG_HOME=str(BARE_STATE / "xdg"))


def loader(path: Path) -> Path:
    done = subprocess.run(
        [ONEPIPELINE, "template", "resolve", "design-doc", "--json"]
        + ["--template-root", CLONE / "templates"],
        capture_output=True,
        cwd=CLONE,
        env=clean_env(),
        check=True,
    )
    path.write_bytes(done.stdout)
    return path


def render_document(state: Path, source: str, env: dict, cwd: Path) -> None:
    """Create the plan's design document in its folder through the store."""
    out = state / "out"
    argv = [STOCK_BINARY, "document", "create", source, "--project", PLAN]
    argv += ["--id", f"{PLAN}-design", "--title", f"Design: {PLAN}"]
    argv += ["--template-loader", loader(state / "loader.json")]
    argv += ["--answers", out / "design-answers.json", "--no-interactive", "--json"]
    argv += ["--asset", out / "plan-dag.png"]
    done = subprocess.run(
        [str(a) for a in argv], env=env, cwd=cwd, capture_output=True, check=False
    )
    if done.returncode:
        sys.exit(
            f"run.py: rendering the document failed: {done.stderr.decode()[-2000:]}"
        )
    try:
        sizes = workload.rendered_sizes(state / "md", PLAN)
    except ValueError as refused:
        sys.exit(f"run.py: the generated workload is not the brief's: {refused}")
    sizes["image"] = [len((out / "plan-dag.png").read_bytes())] * 2
    (state / "rendered-sizes.json").write_text(json.dumps(sizes, indent=1))


def record_workload(run: Path, state: Path) -> None:
    """Keep the sizes this run copied (bytes, characters) and its declarations beside its logs."""
    sizes = state / "rendered-sizes.json"
    if sizes.exists():
        (run / "workload.json").write_text(sizes.read_text())


def change_one(state: Path) -> None:
    """One task's body changed, the document and every other record as they were."""
    variant = int((state / "variant").read_text()) + 1
    workload.rewrite_one(state / "md", PLAN, 1, variant)
    (state / "variant").write_text(str(variant))
    sizes = json.loads((state / "rendered-sizes.json").read_text())
    sizes |= {
        k: v
        for k, v in workload.rendered_sizes(state / "md", PLAN).items()
        if k != "document"
    }
    (state / "rendered-sizes.json").write_text(json.dumps(sizes, indent=1))


def bare_setup(scenario: str) -> None:
    state = BARE_STATE
    if scenario == "bare-first":
        if (state / "md").exists():
            sys.exit(
                f"run.py: {state} already holds a copied plan; run `cleanup` first"
            )
        state.mkdir(parents=True, exist_ok=True)
        bare_config(UNREACHABLE, None)
        workload.main_args(state / "md", PLAN, state / "out", variant=0, rewrite=False)
        render_document(state, "spike-md", bare_env(), state)
        (state / "variant").write_text("0")
    elif not (state / "md").exists():
        sys.exit("run.py: the bare plan has not been copied; run `bare-first` first")
    elif scenario == "bare-changed1":
        change_one(state)


def run_bare(run: Path, scenario: str, proxy: Proxy) -> Path:
    bare_setup(scenario)
    record_workload(run, BARE_STATE)
    bare_config(proxy.endpoint, run)
    env = bare_env() | proxy.env() | {"GH_PROJECTS_TOKEN": token()}
    (run / "conditions.json").write_text(
        json.dumps(conditions(env, BARE_STATE), indent=1)
    )
    exe = binary()
    for verb, record in (("project", PLAN), ("document", f"{PLAN}-design")):
        step(
            run,
            f"{verb}-copy",
            [exe, verb, "copy", f"spike-md:{record}", "--to", "testboard", "--json"],
            env,
            BARE_STATE,
        )
    return BARE_STATE / "store-cache"


HOST_STATE = CACHE / "state" / "host"


def host_env(proxy: Proxy | None, run: Path | None) -> dict[str, str]:
    """The scratch clone's environment: its `authoring` source rooted at the generated folder,
    its `plans` source on the test board behind the proxy, `followups` pointed nowhere."""
    state = HOST_STATE
    env = clean_env(
        ONETASKGRAPH_SOURCES__AUTHORING__CONFIG__ROOT=str(state / "md"),
        ONETASKGRAPH_SOURCES__PLANS__CONFIG__ENDPOINT=proxy.endpoint
        if proxy
        else UNREACHABLE,
        ONETASKGRAPH_SOURCES__PLANS__CONFIG__OWNER=TEST_OWNER,
        ONETASKGRAPH_SOURCES__PLANS__CONFIG__PROJECT_NUMBER=str(TEST_NUMBER),
        ONETASKGRAPH_SOURCES__PLANS__CONFIG__REPOSITORY=TEST_REPOSITORY,
        ONETASKGRAPH_SOURCES__PLANS__CACHE__DIR=str(state / "cache"),
        ONETASKGRAPH_SOURCES__FOLLOWUPS__CONFIG__ENDPOINT=UNREACHABLE,
        GH_PROJECTS_TOKEN=token(),
    )
    # No onetaskgraph user layer: the host's private sources are never built or reached.
    (state / "xdg-empty").mkdir(parents=True, exist_ok=True)
    env["XDG_CONFIG_HOME"] = str(state / "xdg-empty")
    policy = screening(run)
    env["ONETASKGRAPH_WRITE_POLICY__VISIBILITY_COMMAND"] = ",".join(
        policy["visibility_command"]
    )
    env["ONETASKGRAPH_WRITE_POLICY__CHECK_COMMAND"] = ",".join(policy["check_command"])
    if proxy:
        env |= proxy.env()
    if run:
        env["SPIKE_HOST_AUDIT"] = str(run / "host.jsonl")
        env["PYTHONPATH"] = str(HERE / "hostaudit")
    return env


def host_setup() -> None:
    state = HOST_STATE
    if (state / "md").exists():
        return
    state.mkdir(parents=True, exist_ok=True)
    workload.main_args(
        state / "md",
        PLAN,
        state / "out",
        variant=0,
        rewrite=False,
        last_task_without_repositories=True,
    )
    (state / "variant").write_text("0")
    env = host_env(None, None)
    render_document(state, "authoring", env, CLONE)
    stamp(env)


def stamp(env: dict) -> None:
    """Stamp the host plan's review records for its current content (no judged turn)."""
    argv = ["uv", "run", "python", HERE / "stamp_reviews.py", f"authoring:{PLAN}"]
    clean = {
        k: v for k, v in env.items() if k not in ("SPIKE_HOST_AUDIT", "PYTHONPATH")
    }
    done = subprocess.run(
        [str(a) for a in argv], env=clean, cwd=CLONE, capture_output=True, check=False
    )
    if done.returncode:
        sys.exit(
            f"run.py: stamping review records failed: {done.stderr.decode()[-3000:]}"
        )


def board_project(state: Path, source: str) -> str:
    """The board id a copy recorded on the local project, as `<source>:<id>`."""
    text = (state / "md" / "projects" / f"{PLAN}.md").read_text()
    match = re.search(r'"onetaskgraph\.copies":\s*(\{.*\})', text)
    if not match:
        sys.exit(f"run.py: {PLAN} records no copy; run the copy scenario first")
    return json.loads(match.group(1))[source]


def run_host(run: Path, scenario: str, proxy: Proxy) -> Path:
    if scenario == "host-first" and (HOST_STATE / "md").exists():
        sys.exit(
            f"run.py: {HOST_STATE} already holds a copied plan; run `cleanup` first"
        )
    if scenario != "host-first" and not (HOST_STATE / "md").exists():
        sys.exit("run.py: the host plan has not been copied; run `host-first` first")
    host_setup()
    env = host_env(proxy, run)
    if scenario == "host-changed1":
        change_one(HOST_STATE)
        stamp(env)
    record_workload(run, HOST_STATE)
    (run / "conditions.json").write_text(json.dumps(conditions(env, CLONE), indent=1))
    if scenario == "host-approve":
        target = board_project(HOST_STATE, "plans")
        step(run, "approve-design", ["just", "approve-design", target], env, CLONE)
    else:
        step(run, "copy-plan", ["just", "copy-plan", f"authoring:{PLAN}"], env, CLONE)
    return HOST_STATE / "cache"


def last_headers() -> dict | None:
    """The GraphQL bucket as the last real answer any run recorded reported it, while its
    window is still open: `{limit, used, remaining, reset, at}`, or `None`."""
    logs = sorted(RUNS.glob("*/proxy.jsonl"), key=lambda p: p.stat().st_mtime)
    for log in reversed(logs[-3:]):
        records = [
            json.loads(line) for line in log.read_text().splitlines() if line.strip()
        ]
        graphql = [
            r for r in records if r["limits"].get("x-ratelimit-resource") == "graphql"
        ]
        if not graphql:
            continue
        last = max(graphql, key=lambda r: r["end"])
        reset = int(last["limits"]["x-ratelimit-reset"])
        if reset <= time.time():
            return None
        return {
            "limit": int(last["limits"]["x-ratelimit-limit"]),
            "used": int(last["limits"]["x-ratelimit-used"]),
            "remaining": int(last["limits"]["x-ratelimit-remaining"]),
            "reset": reset,
            "at": last["end"],
        }
    return None


def corrected_read(token_value: str) -> dict:
    """The corrected gate's reading: the least of `GET /rate_limit`, three GraphQL
    `rateLimit` reads and the last real answer's `x-ratelimit-*` headers."""
    out = quota.read(token_value)
    headers = last_headers()
    if headers is not None:
        out["graphql (last headers)"] = {
            k: headers[k] for k in ("limit", "used", "remaining", "reset")
        }
        if headers["remaining"] < out["graphql"]["remaining"]:
            out["graphql"] = dict(out["graphql (last headers)"])
    return out


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("scenario", choices=SCENARIOS)
    parser.add_argument("--no-sanitize", action="store_true")
    parser.add_argument(
        "--only",
        choices=("bare", "host"),
        help="cleanup: only the issues one side's runs created",
    )
    args = parser.parse_args()
    binary()
    stamp_at = time.strftime("%Y%m%dT%H%M%S", time.gmtime())
    name = args.scenario
    run = RUNS / f"{name}-{stamp_at}"
    run.mkdir(parents=True)
    print(f"{name}: raw logs in {run}", flush=True)
    (run / "versions.json").write_text(json.dumps(versions(), indent=1))
    logged = len(QUOTA_LOG.read_text().splitlines()) if QUOTA_LOG.exists() else 0
    reading = quota.gate(token(), QUOTA_LOG, name, quota.BUCKETS, reader=corrected_read)
    events = [
        json.loads(line)
        for line in QUOTA_LOG.read_text().splitlines()[logged:]
        if line.strip()
    ]
    (run / "quota-events.json").write_text(json.dumps(events))
    (run / "quota-before.json").write_text(json.dumps(reading))
    (run / "burn-before.json").write_text(json.dumps(quota.burn(token())))
    proxy = Proxy(run, board_ids())
    cache_dir: Path | None = None
    try:
        if args.scenario in BARE:
            cache_dir = BARE_STATE / "store-cache"
            (run / "cache-before.json").write_text(json.dumps(cache_state(cache_dir)))
            run_bare(run, args.scenario, proxy)
        elif args.scenario in HOST:
            cache_dir = HOST_STATE / "cache"
            (run / "cache-before.json").write_text(json.dumps(cache_state(cache_dir)))
            run_host(run, args.scenario, proxy)
        else:
            import cleanup

            cleanup.run(run, proxy.endpoint, token(), CACHE / "state", args.only)
    finally:
        proxy.stop()
        after = quota.read(token())
        (run / "quota-after.json").write_text(
            json.dumps({"at": time.time(), "buckets": after})
        )
        (run / "burn-after.json").write_text(json.dumps(quota.burn(token())))
        if cache_dir is not None:
            (run / "cache-after.json").write_text(json.dumps(cache_state(cache_dir)))
    if not args.no_sanitize:
        out = sanitize.write(run, LOGS / f"{name}-{stamp_at}")
        print(f"{name}: sanitized logs in {out.relative_to(REPO)}", flush=True)
    if FAILED_STEPS:
        sys.exit(
            f"run.py: {', '.join(FAILED_STEPS)} failed (its stderr is above and in {run}); this "
            "capture is not a row: move it to captures/superseded/ and take the scenario again"
        )


if __name__ == "__main__":
    main()
