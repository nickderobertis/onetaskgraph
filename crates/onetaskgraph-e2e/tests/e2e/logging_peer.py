"""A stdio plugin that is another plugin, with every method it is sent written down.

    python3 logging_peer.py LOG COMMAND [ARGUMENT...]

It starts COMMAND — the shipped host, `onetaskgraph plugin-serve <kind>` — hands it every line
the engine sends, and hands the engine every line the host answers, unchanged. Before forwarding
a request it appends the request's method to LOG, one per line, so a journey can say exactly
which methods the peer was sent and that none followed a refusal. The engine spawns a plugin
with a cleared environment (§3.1), so both paths arrive as arguments.
"""

# llmlint: ignore-file[modern_domain_modeling] A pipe between two processes that forwards JSON
# lines it does not interpret: there is no domain here to model, and it is spawned by whichever
# `python3` the host has, so it may import nothing outside the standard library.

import json
import subprocess
import sys
import threading


def relay(source, sink):
    """Copy every line of `source` to `sink`, flushing each, until `source` ends."""
    for line in source:
        sink.write(line)
        sink.flush()


def main():
    log, command = sys.argv[1], sys.argv[2:]
    child = subprocess.Popen(
        command,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        bufsize=1,
    )
    answers = threading.Thread(target=relay, args=(child.stdout, sys.stdout), daemon=True)
    answers.start()
    for line in sys.stdin:
        try:
            method = json.loads(line).get("method", "")
        except ValueError:
            method = ""
        with open(log, "a", encoding="utf-8") as handle:
            handle.write("%s\n" % method)
        child.stdin.write(line)
        child.stdin.flush()
    child.stdin.close()
    child.wait()
    answers.join()


if __name__ == "__main__":
    main()
