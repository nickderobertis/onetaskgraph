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

# llmlint: ignore-file[async_typed_clients_at_boundaries] The protocol is one request and then
# its answer (§2), so this relay holds exactly one line in flight at a time: it writes a request
# to the host, reads that request's answer, and only then reads the engine's next line. There is
# nothing to overlap, and an event loop would add a dependency on the host's `asyncio` behaviour
# to a script the engine spawns with a cleared environment (§3.1) for no request it could serve
# sooner.

import json
import subprocess
import sys
from dataclasses import dataclass


@dataclass(frozen=True)
class Rewrite:
    """The relay's own settings, taken out of the `initialize` request before it is forwarded."""

    host: str
    after: str
    skip: int
    file: str
    text: str

    @classmethod
    def taken_from(cls, settings):
        """Remove the relay's members from `settings`, leaving what the host is initialized with."""
        return cls(
            host=str(settings.pop("host")),
            after=str(settings.pop("after")),
            skip=int(settings.pop("skip")),
            file=str(settings.pop("file")),
            text=str(settings.pop("text")),
        )


def main():
    first = json.loads(sys.stdin.readline())
    rewrite = Rewrite.taken_from(first["params"]["config"])
    host = subprocess.Popen(
        [rewrite.host],
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
        return host.stdout.readline()

    sys.stdout.write(relayed(json.dumps(first) + "\n"))
    sys.stdout.flush()
    for line in sys.stdin:
        method = json.loads(line).get("method")
        answer = relayed(line)
        if method == rewrite.after and not written:
            if answered == rewrite.skip:
                with open(rewrite.file, "w", encoding="utf-8", newline="\n") as rewritten:
                    rewritten.write(rewrite.text)
                written = True
            answered += 1
        sys.stdout.write(answer)
        sys.stdout.flush()
    host.stdin.close()
    host.wait()


if __name__ == "__main__":
    main()
