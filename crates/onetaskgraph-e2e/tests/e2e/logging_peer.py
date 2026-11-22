"""A stdio plugin that is another plugin, with every method it is sent written down.

    python3 logging_peer.py LOG [--declare MEMBER | --withhold MEMBER]... COMMAND [ARGUMENT...]

It starts COMMAND — the shipped host, `onetaskgraph plugin-serve <kind>` — hands it every line
the engine sends, and hands the engine every line the host answers, unchanged. Before forwarding
a request it appends the request's method to LOG, one per line, so a journey can say exactly
which methods the peer was sent and that none followed a refusal — an `update_task` naming
`duplicate_of` written as `update_task duplicate_of`. Each `--declare MEMBER` adds `MEMBER: true`
to the host's handshake answer and each `--withhold MEMBER` removes `MEMBER` from it, so a journey
can stand up a peer declaring what this build's own host does not, or not what it does. The
engine spawns a plugin with a cleared environment (§3.1), so every path arrives as an argument.
"""

# llmlint: ignore-file[async_typed_clients_at_boundaries] No network or shared service is
# reached here: this relays the lines of two local pipes between the engine and the host it
# spawned, and it is spawned by whichever `python3` the host has, so it keeps to the standard
# library's blocking pipes and one thread per direction, as the document-store peer beside it does.
# llmlint: ignore-file[modern_domain_modeling] A pipe between two processes that forwards JSON
# lines it does not interpret: there is no domain here to model, and it is spawned by whichever
# `python3` the host has, so it may import nothing outside the standard library.

import json
import subprocess
import sys
import threading
from typing import TextIO


def relay(source: TextIO, sink: TextIO, declared: list[str], withheld: list[str]) -> None:
    """Copy every line of `source` to `sink`, flushing each, until `source` ends.

    The first line, the handshake's answer, carries each of `declared` added to its result and
    each of `withheld` removed from it.
    """
    first = True
    for line in source:
        if first and (declared or withheld):
            answer = json.loads(line)
            result = answer.setdefault("result", {})
            for member in declared:
                result[member] = True
            for member in withheld:
                result.pop(member, None)
            line = json.dumps(answer) + "\n"
        first = False
        sink.write(line)
        sink.flush()


def main() -> None:
    """Forward the engine's requests to the host, each method written down first."""
    log, command = sys.argv[1], sys.argv[2:]
    declared, withheld = [], []
    while command[:1] in (["--declare"], ["--withhold"]):
        (declared if command[0] == "--declare" else withheld).append(command[1])
        command = command[2:]
    child = subprocess.Popen(
        command,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        bufsize=1,
    )
    answers = threading.Thread(
        target=relay, args=(child.stdout, sys.stdout, declared, withheld), daemon=True
    )
    answers.start()
    for line in sys.stdin:
        try:
            request = json.loads(line)
            method = request.get("method", "")
            if method == "update_task" and "duplicate_of" in json.dumps(request.get("params")):
                method = "update_task duplicate_of"
        except ValueError:
            method = ""
        with open(log, "a", encoding="utf-8") as handle:
            handle.write(f"{method}\n")
        child.stdin.write(line)
        child.stdin.flush()
    child.stdin.close()
    child.wait()
    answers.join()


if __name__ == "__main__":
    main()
