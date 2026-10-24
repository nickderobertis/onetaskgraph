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
env = dict(os.environ, XDG_CONFIG_HOME=xdg)
start = time.time()
code = subprocess.run([onevcs, *args], input=data, env=env, check=False).returncode
log = LOG if LOG != "-" else None
if log:
    with open(log, "a") as out:
        out.write(json.dumps({"kind": "screen", "name": args[1], "start": start, "end": time.time(), "returncode": code}) + "\n")
sys.exit(code)
