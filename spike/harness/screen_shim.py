"""A `write_policy` command that runs the real onevcs and records the call.

Usage: python3 screen_shim.py <log> <onevcs> boundary (inspect|check) ...

The input is passed through unchanged. Each call appends `{name, start, end, returncode,
input_sha256, input}` to `<log>`; the input is kept only for synthetic cases, whose sandbox
holds no real work, so the union experiment can replay exactly what the store sent.
"""

import hashlib
import json
import subprocess
import sys
import time

log, onevcs, *args = sys.argv[1:]
data = sys.stdin.buffer.read()
start = time.time()
done = subprocess.run([onevcs, *args], input=data, capture_output=True, check=False)
end = time.time()
sys.stdout.buffer.write(done.stdout)
sys.stderr.buffer.write(done.stderr)
with open(log, "a") as out:
    record = {
        "kind": "screen",
        "name": args[1],
        "start": start,
        "end": end,
        "returncode": done.returncode,
        "input_sha256": hashlib.sha256(data).hexdigest(),
        "input": data.decode(errors="replace"),
        "answer": done.stdout.decode(errors="replace")[:400],
    }
    out.write(json.dumps(record) + "\n")
sys.exit(done.returncode)
