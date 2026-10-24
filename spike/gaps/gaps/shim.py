"""A `write_policy` command that runs the real onevcs and records the call.

`python -m gaps.shim <log> <onevcs> boundary (inspect|check) [args…]`

Standard input is handed to onevcs byte for byte, and its standard output, standard error and
exit status come back unchanged. Each call appends one `ScreenCall` to `<log>`, the input
included: only synthetic cases use this shim, and their sandbox holds no real work, so the
union experiment can replay exactly what the store sent. Exit status 64 is this shim's own:
it was invoked without a `boundary inspect` or `boundary check` command to run.
"""

from __future__ import annotations

import asyncio
import hashlib
import json
import sys
import time
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path

from gaps import proc
from gaps.json_shape import Json, decode, encode

USAGE = 64


class Command(StrEnum):
    """The boundary command a call ran."""

    INSPECT = "inspect"
    CHECK = "check"


@dataclass(frozen=True)
class ScreenCall:
    """One recorded `write_policy` call."""

    name: Command
    start: float
    end: float
    returncode: int
    input_sha256: str
    input: str

    def to_json(self) -> Json:
        """The call as one log line."""
        return encode(self)

    @classmethod
    def parse(cls, value: Json, where: str) -> ScreenCall:
        """A logged call, validated."""
        return decode(cls, value, where)


async def shim(log: Path, onevcs: Path, args: list[str], data: bytes) -> int:
    """Run onevcs once, pass its output through and record the call; answer its exit status."""
    command = Command(args[1])
    start = time.time()
    done = await proc.run([onevcs, *args], stdin=data)
    sys.stdout.buffer.write(done.stdout)
    sys.stderr.buffer.write(done.stderr)
    call = ScreenCall(
        name=command,
        start=start,
        end=start + done.seconds,
        returncode=done.returncode,
        input_sha256=hashlib.sha256(data).hexdigest(),
        input=data.decode(errors="replace"),
    )
    with log.open("a", encoding="utf-8") as out:
        out.write(json.dumps(call.to_json(), sort_keys=True) + "\n")
    return done.returncode


def main() -> None:
    """Validate the invocation, then shim one call."""
    words = sys.argv[1:]
    if len(words) < 4 or words[2] != "boundary" or words[3] not in {c.value for c in Command}:
        sys.stderr.write("usage: python -m gaps.shim <log> <onevcs> boundary (inspect|check) …\n")
        sys.exit(USAGE)
    log, onevcs, args = Path(words[0]), Path(words[1]), words[2:]
    sys.exit(asyncio.run(shim(log, onevcs, args, sys.stdin.buffer.read())))


if __name__ == "__main__":
    main()
