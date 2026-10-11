"""A stdio relay in front of the shipped plugin host, which can rewrite one file part way through
and can make the host read as a plugin written before a protocol member existed.

A folder of Markdown has no server a journey can make wait between two of a command's reads, so
a person saving a task file while a copy is reading it cannot be staged in process. This relay
is that person. It is configured as a `subprocess` source whose settings name the real host —
`onetaskgraph-source` — and the plugin the host serves, which it hands on unchanged; it forwards
every line between the engine and the host, and once the host has answered the method named by
`after` for the `skip + 1`th time, it writes `text` to `file` before the engine reads that
answer. The engine's next request therefore meets the rewritten file, and nothing else differs
from a source the engine reaches over the protocol (`docs/plugin-protocol.md`).

It is also an older peer: each member `forget` names is removed from the host's `initialize`
answer, so the engine reads the handshake of a plugin written before that member existed and
must never send it the method the member declares — which `log` lets a journey prove, as the
name of every method the engine sent, one per line.

Its settings, handed over in the `initialize` request (§3), are
`{"host": <path>, "kind": <plugin>, "config": <its block>}`, the last two being what the host is
initialized with, and optionally `"after": <method>, "skip": <n>, "file": <path>,
"text": <string>` together, `"forget": [<member>, ...]` and `"log": <path>`.

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

from __future__ import annotations

import json
import subprocess
import sys
from dataclasses import dataclass


def text_setting(settings, member):
    """The string `settings` names as `member`, or a refusal naming it."""
    value = settings.pop(member, None)
    if not isinstance(value, str) or not value:
        raise SystemExit(
            f"rewriting relay: setting {member!r} must be a non-empty string, got {value!r}"
        )
    return value


@dataclass(frozen=True)
class Rewrite:
    """The one rewrite the relay makes: `text` into `file`, once `after` is answered `skip + 1` times."""

    after: str
    skip: int
    file: str
    text: str

    @classmethod
    def taken_from(cls, settings):
        """Remove the rewrite's members from `settings`, or `None` when it names no rewrite."""
        if "after" not in settings:
            return None
        skip = settings.pop("skip", None)
        # `bool` is an `int` to Python, and no journey means `true` by a count.
        if not isinstance(skip, int) or isinstance(skip, bool) or skip < 0:
            raise SystemExit(
                f"rewriting relay: setting 'skip' must be a count of zero or more, got {skip!r}"
            )
        text = settings.pop("text", None)
        if not isinstance(text, str):
            raise SystemExit(
                f"rewriting relay: setting 'text' must be a string, got {text!r}"
            )
        return cls(
            after=text_setting(settings, "after"),
            skip=skip,
            file=text_setting(settings, "file"),
            text=text,
        )


@dataclass(frozen=True)
class Relay:
    """The relay's own settings, taken out of the `initialize` request before it is forwarded."""

    host: str
    rewrite: Rewrite | None
    forget: tuple[str, ...]
    log: str | None

    @classmethod
    def taken_from(cls, settings):
        """Remove the relay's members from `settings`, leaving what the host is initialized with.

        Each is refused naming it, before the host is started or a file is opened, when it is
        not what a journey writes: a journey misconfiguring its relay fails as that, rather than
        as the host or the copy misbehaving.
        """
        forget = settings.pop("forget", [])
        if not isinstance(forget, list) or not all(
            isinstance(member, str) and member for member in forget
        ):
            raise SystemExit(
                f"rewriting relay: setting 'forget' must be a list of member names, got {forget!r}"
            )
        log = text_setting(settings, "log") if "log" in settings else None
        return cls(
            host=text_setting(settings, "host"),
            rewrite=Rewrite.taken_from(settings),
            forget=tuple(forget),
            log=log,
        )


def message(line, sender):
    """One protocol line as the JSON object every line is (§1), or a refusal naming `sender`."""
    try:
        decoded = json.loads(line)
    except ValueError as error:
        raise SystemExit(
            f"rewriting relay: {sender} sent a line that is not JSON: {error}"
        ) from error
    if not isinstance(decoded, dict):
        raise SystemExit(
            f"rewriting relay: {sender} sent {decoded!r}, which is not a JSON object"
        )
    return decoded


def main():
    first = message(sys.stdin.readline(), "the engine")
    params = first.get("params")
    config = params.get("config") if isinstance(params, dict) else None
    if first.get("method") != "initialize" or not isinstance(config, dict):
        raise SystemExit(
            "rewriting relay: the engine's first line must be an `initialize` request whose "
            f"params carry a `config` object, got {first!r}"
        )
    relay = Relay.taken_from(config)
    host = subprocess.Popen(
        [relay.host],
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

    def logged(method):
        if relay.log is not None:
            with open(relay.log, "a", encoding="utf-8", newline="\n") as log:
                log.write(f"{method}\n")

    logged(first.get("method"))
    handshake = message(relayed(json.dumps(first) + "\n"), "the host")
    result = handshake.get("result")
    if isinstance(result, dict):
        for member in relay.forget:
            result.pop(member, None)
    sys.stdout.write(json.dumps(handshake) + "\n")
    sys.stdout.flush()
    for line in sys.stdin:
        method = message(line, "the engine").get("method")
        if not isinstance(method, str):
            raise SystemExit(
                f"rewriting relay: the engine sent a request naming no method: {line!r}"
            )
        logged(method)
        answer = relayed(line)
        rewrite = relay.rewrite
        if rewrite is not None and method == rewrite.after and not written:
            if answered == rewrite.skip:
                with open(
                    rewrite.file, "w", encoding="utf-8", newline="\n"
                ) as rewritten:
                    rewritten.write(rewrite.text)
                written = True
            answered += 1
        sys.stdout.write(answer)
        sys.stdout.flush()
    host.stdin.close()
    host.wait()


if __name__ == "__main__":
    main()
