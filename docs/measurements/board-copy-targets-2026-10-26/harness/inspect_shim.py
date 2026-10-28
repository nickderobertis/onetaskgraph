# llmlint: ignore-file[async_typed_clients_at_boundaries] The onevcs call is the host's own write_policy command run as a subprocess, exactly as the store runs it; an async client would not be the command the store invokes.
# llmlint: ignore-file[new_code_lands_in_a_project] This is the measurement record beside the captures it produced: run by hand against GitHub through measure.sh, never by a target. docs/measurements is no Nx project (nor is coalesced-members-dry-run/dry_run.py beside it), and making it one would put a live-GitHub harness inside affected selection.
# llmlint: ignore-file[boundary_inputs_validated] Every input here is this harness's own raw log, its own state folder or the plan it generated moments earlier; the committed captures are validated where they are read, by analyze.py, and what leaves the raw logs is held by the sanitizer's allowlist and leak scan.
# llmlint: ignore-file[changed_behavior_has_e2e] Its proof is the live run itself, whose captures are committed and reconciled by test_analyze.py; the GitHub-facing parts can be driven only against GitHub's test board, which measure.sh does, and the sanitizer produces every capture those tests read.
# llmlint: ignore-file[contracts_have_one_source_or_a_drift_gate] What this mirrors (the plugin's documents, the host's configuration, the scratch identities) is pinned to the releases measured, onetaskgraph v0.12.7 and ai-orchestrator 668dc496, each stated in the report and in every row's versions.json; the capture schema it shares with analyze.py is held by test_analyze.py, which builds every capture through sanitize.write.
# llmlint: ignore-file[modern_domain_modeling] These records are JSON lines passed by allowlist from the proxy's raw log to the sanitized captures; the typed reading of them is analyze.py, and remodelling the retained harness would make it no longer the code that took the captures.
"""The `write_policy` commands of the scratch host process, both through the real onevcs.

The scratch process runs with no onetaskgraph user layer (its XDG_CONFIG_HOME is an empty
directory), so onevcs is run with the host's own configuration home, `--xdg`, and sees this
host's registrations as the real host's store would. This host's onevcs has not registered the
test board's scratch repository (public on GitHub), so an inspection of it is answered by
inspecting a registered public repository of the same owner, keeping the subprocess and its
refresh; every other request is passed through unchanged.

Usage: python3 inspect_shim.py --xdg <dir> --log <file> <onevcs> boundary (inspect|check) ...
"""

import json
import os
import subprocess
import sys
import time

SCRATCH = "nickderobertis/onetaskgraph-live-scratch"
STAND_IN = "github.com/nickderobertis/onetaskgraph"

_, xdg, _, LOG, onevcs, *args = sys.argv[1:]
data = sys.stdin.buffer.read()
if args[:2] == ["boundary", "inspect"]:
    request = json.loads(data)
    named = str(request.get("repository", "")).strip().lower().removeprefix("https://")
    if named.removeprefix("github.com/") == SCRATCH:
        request["repository"] = STAND_IN
    data = json.dumps(request).encode()
# The host's timing audit stays in the host's own process: onevcs is not a step of it.
env = {
    k: v for k, v in os.environ.items() if k not in ("PYTHONPATH", "SPIKE_HOST_AUDIT")
}
env["XDG_CONFIG_HOME"] = xdg
start = time.time()
code = subprocess.run([onevcs, *args], input=data, env=env, check=False).returncode
log = LOG if LOG != "-" else None
if log:
    with open(log, "a") as out:
        out.write(
            json.dumps(
                {
                    "kind": "screen",
                    "name": args[1],
                    "start": start,
                    "end": time.time(),
                    "returncode": code,
                }
            )
            + "\n"
        )
sys.exit(code)
