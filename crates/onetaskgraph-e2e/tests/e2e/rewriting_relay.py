"""A stdio relay in front of the shipped plugin host that rewrites one file, once, part way through.

A folder of Markdown has no server a journey can make wait between two of a command's reads, so
a person saving a task file while a copy is reading it cannot be staged in process. This relay
is that person. It is configured as a `subprocess` source whose settings name the real host —
`onetaskgraph-source` — and the plugin the host serves, which it hands on unchanged; it forwards
every line between the engine and the host, and once the host has answered the method named by
`after` for the `skip + 1`th time, it writes `text` to `file` before the engine reads that
answer. The engine's next request therefore meets the rewritten file, and nothing else differs
from a source the engine reaches over the protocol (`docs/plugin-protocol.md`).

Its settings, handed over in the `initialize` request (§3), are
`{"host": <path>, "after": <method>, "skip": <n>, "file": <path>, "text": <string>,
  "kind": <plugin>, "config": <its block>}`; the last two are what the host is initialized with.

It imports nothing outside the standard library and uses no syntax newer than Python 3.8,
because the engine spawns it with a cleared environment (§3.1) through whichever `python3` or
`python` the host has.
"""

import json
import subprocess
import sys

RELAY_KEYS = ("host", "after", "skip", "file", "text")


def main():
    first = json.loads(sys.stdin.readline())
    settings = first["params"]["config"]
    relay = {key: settings.pop(key) for key in RELAY_KEYS}
    host = subprocess.Popen(
        [relay["host"]],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        universal_newlines=True,
        encoding="utf-8",
    )
    answered = 0
    written = False

    def relayed(line):
        host.stdin.write(line)
        host.stdin.flush()
        answer = host.stdout.readline()
        return answer

    sys.stdout.write(relayed(json.dumps(first) + "\n"))
    sys.stdout.flush()
    for line in sys.stdin:
        method = json.loads(line).get("method")
        answer = relayed(line)
        if method == relay["after"] and not written:
            if answered == relay["skip"]:
                with open(relay["file"], "w", encoding="utf-8", newline="\n") as rewritten:
                    rewritten.write(relay["text"])
                written = True
            answered += 1
        sys.stdout.write(answer)
        sys.stdout.flush()
    host.stdin.close()
    host.wait()


if __name__ == "__main__":
    main()
