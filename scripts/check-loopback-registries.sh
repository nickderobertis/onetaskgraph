#!/usr/bin/env bash
# Prove the two release checks' loopback registries bind without a reverse DNS lookup.
#
# `http.server.HTTPServer.server_bind` names itself with `socket.getfqdn(host)`, and on the
# macOS release runner that lookup of 127.0.0.1 outlasted the whole start-up window — so a
# registry that had in fact bound was reported as one that never reported a port, with an
# empty log where the reason belonged. scripts/loopback-crate-registry.py and
# scripts/loopback-npm-registry.py both bind without it for that reason, and nothing about
# the code that meets that requirement makes it visible: a cleanup to `python3 -m
# http.server` passes every Linux check and hangs the macOS release lane.
#
# So this drives both real launchers through a python whose `socket.getfqdn` never answers,
# and reads the port each one writes. Case 1 is the stock bind the launchers refuse, run
# through the same shim: it has to be caught, because a shim that did not take would leave
# every case below passing on the one platform they cannot fail on.
set -euo pipefail

fatal() {
  echo "check-loopback-registries: $1" >&2
  echo "check-loopback-registries: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just script-check' does"
readonly ROOT

command -v python3 >/dev/null 2>&1 || fatal \
  "python3 is not on PATH, and both registries under test are python" \
  "install it (scripts/provision-gate.sh names what this workspace needs), then rerun"

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree this check runs its registries from" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
started_pids=""
cleanup() {
  # Only the processes this check started, by the pids it was given.
  for pid in $started_pids; do kill "$pid" 2>/dev/null || true; done
  rm -rf "$scratch"
}
trap cleanup EXIT

mkdir -p "$scratch/shim" "$scratch/index" || fatal \
  "could not create the scratch tree's directories" "check \$TMPDIR permissions, then rerun"

# Every python3 below imports this through PYTHONPATH, and it does one thing: leave
# `socket.getfqdn` a name resolver that records the call and then never answers, which is
# what the macOS runner's was. Reaching it through the interpreter's own startup rather
# than through an edit to either launcher is what makes this the REAL launcher under test.
cat > "$scratch/shim/sitecustomize.py" <<'PY' || fatal \
  "could not write the resolver shim in $scratch, so nothing would be simulated" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
"""`socket.getfqdn` as a resolver that never answers, and nothing else."""

import os
import socket
import time

_record = os.environ.get("ONETASKGRAPH_GETFQDN_CALLED")


def _getfqdn(name=""):
    if _record:
        with open(_record, "w", encoding="utf-8") as handle:
            handle.write(f"socket.getfqdn({name!r})\n")
    while True:  # The macOS lookup came back eventually; nothing here waits that long.
        time.sleep(3600)


_getfqdn.onetaskgraph_blocks = True
socket.getfqdn = _getfqdn
PY

if [ "$(PYTHONPATH="$scratch/shim" python3 -c \
  'import socket; print(getattr(socket.getfqdn, "onetaskgraph_blocks", False))' | tr -d '\r')" != "True" ]; then
  fatal \
    "the resolver shim did not take: python3 still holds its own socket.getfqdn, so every case below would pass without simulating anything" \
    "run 'PYTHONPATH=$scratch/shim python3 -c \"import socket\"' and fix what it reports, then rerun"
fi

# A registry bound the stock way, which is what a cleanup to `python3 -m http.server` would
# leave behind. It exists to be caught: it reports no port under the shim, and case 1 fails
# if it ever does.
cat > "$scratch/stock-registry.py" <<'PY' || fatal \
  "could not write the stock stand-in in $scratch" "check \$TMPDIR permissions, then rerun"
"""A loopback registry bound the way http.server binds one, for this check to catch."""

import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

port_file = sys.argv[1]
server = HTTPServer(("127.0.0.1", 0), BaseHTTPRequestHandler)
with open(port_file, "w", encoding="utf-8") as handle:
    handle.write(str(server.server_address[1]))
server.serve_forever()
PY

failures=0
fail() {
  echo "check-loopback-registries: FAILED ($1)" >&2
  failures=$((failures + 1))
}

port_file=""
called_file=""
log_file=""
case_pid=""

launch() { # the launcher, then its own arguments — the port file among them
  rm -f "$port_file" "$called_file"
  PYTHONPATH="$scratch/shim" ONETASKGRAPH_GETFQDN_CALLED="$called_file" \
    python3 "$@" >"$log_file" 2>&1 &
  case_pid=$!
  started_pids="$started_pids $case_pid"
}

# Thirty seconds, which is what the two checks under test give these very servers.
await() { # the file to wait for; answers whether it arrived
  local target="$1" tick=0
  while [ "$tick" -lt 300 ]; do
    [ -s "$target" ] && return 0
    kill -0 "$case_pid" 2>/dev/null || break
    sleep 0.1
    tick=$((tick + 1))
  done
  [ -s "$target" ]
}

stop() {
  [ -z "$case_pid" ] || kill "$case_pid" 2>/dev/null || true
  case_pid=""
}

# What a port file holds is a port only because the registry put it there, so a case that
# reads one says which it got.
reports_a_port() {
  local reported
  reported="$(cat "$port_file" 2>/dev/null | tr -d '\r\n')"
  case $reported in
    '' | *[!0-9]*) return 1 ;;
  esac
  [ "$reported" -ge 1 ] && [ "$reported" -le 65535 ]
}

# 1. The stock bind, caught. The shim records the call and then holds the bind for ever, so
#    a registry that binds that way never reaches the line that writes its port.
port_file="$scratch/stock.port"
called_file="$scratch/stock.getfqdn"
log_file="$scratch/stock.log"
launch "$scratch/stock-registry.py" "$port_file"
if ! await "$called_file"; then
  stop
  # The stand-in's own output rather than the path it is at: that path is inside the
  # scratch tree the EXIT trap removes, so naming it would send the reader somewhere that
  # no longer exists by the time they look.
  echo "check-loopback-registries: the stock stand-in neither reported the reverse lookup" >&2
  echo "check-loopback-registries: nor exited saying why, so this check cannot tell a" >&2
  echo "check-loopback-registries: launcher that binds without the lookup from one that" >&2
  echo "check-loopback-registries: does. It said:" >&2
  sed 's/^/    /' <"$log_file" >&2
  echo "check-loopback-registries: next: that stand-in is written by this check itself, as" >&2
  echo "check-loopback-registries: is the resolver shim it runs under — repair whichever of" >&2
  echo "check-loopback-registries: the two its output above names, then rerun." >&2
  exit 1
fi
if reports_a_port; then
  fail "a registry bound the stock way reported a port although socket.getfqdn never answered — so cases 2 and 3 below would pass for a launcher that binds through the reverse lookup, which is the whole of what they are for"
fi
stop

# 2. The sparse crate index scripts/check-crate-sibling-resolution.sh resolves against.
port_file="$scratch/crate.port"
called_file="$scratch/crate.getfqdn"
log_file="$scratch/crate.log"
launch "$ROOT/scripts/loopback-crate-registry.py" "$port_file" "$scratch/index"
if ! await "$port_file"; then
  fail "scripts/loopback-crate-registry.py reported no port within 30s while socket.getfqdn never answered — which is the macOS release runner, where check-crate-sibling-resolution reads an empty log and fails a release. It said:"
  sed 's/^/    /' <"$log_file" >&2
elif ! reports_a_port; then
  fail "scripts/loopback-crate-registry.py wrote '$(cat "$port_file" | tr -d '\r\n')' where a port number belongs"
fi
if [ -e "$called_file" ]; then
  fail "scripts/loopback-crate-registry.py called $(cat "$called_file" | tr -d '\r\n') on its way to a port; on the macOS runner that call is what never came back"
fi
stop

# 3. The npm registry scripts/check-npm-publish.sh publishes into, with the two files it
#    keeps its state in beside the port file it writes.
port_file="$scratch/npm.port"
called_file="$scratch/npm.getfqdn"
log_file="$scratch/npm.log"
printf 'record\n' > "$scratch/npm.mode"
launch "$ROOT/scripts/loopback-npm-registry.py" "$scratch/npm.published" "$port_file" "$scratch/npm.mode"
if ! await "$port_file"; then
  fail "scripts/loopback-npm-registry.py reported no port within 30s while socket.getfqdn never answered — which is the macOS release runner, where check-npm-publish reads an empty log and the whole install path fails on a name nothing reads. It said:"
  sed 's/^/    /' <"$log_file" >&2
elif ! reports_a_port; then
  fail "scripts/loopback-npm-registry.py wrote '$(cat "$port_file" | tr -d '\r\n')' where a port number belongs"
fi
if [ -e "$called_file" ]; then
  fail "scripts/loopback-npm-registry.py called $(cat "$called_file" | tr -d '\r\n') on its way to a port; on the macOS runner that call is what never came back"
fi
stop

# 4. What the cases above prove is about those two checks only while those two checks are
#    what stands these registries up. So each one names its launcher, and neither builds a
#    server of its own for the cases above to miss.
while read -r check launcher; do
  grep -Fq "$launcher" "$ROOT/$check" || fail \
    "$check no longer names $launcher, so the launcher this check proved is not the registry that check starts"
  for inlined in 'server_bind' 'HTTPServer(' 'ThreadingHTTPServer('; do
    if grep -Fq "$inlined" "$ROOT/$check"; then
      fail "$check builds a loopback server of its own ('$inlined'), which nothing above drives — move it into $launcher, which this check holds to binding without the reverse lookup"
    fi
  done
done <<'PAIRS'
scripts/check-crate-sibling-resolution.sh scripts/loopback-crate-registry.py
scripts/check-npm-publish.sh scripts/loopback-npm-registry.py
PAIRS

if [ "$failures" -ne 0 ]; then
  echo "check-loopback-registries: $failures expectation(s) failed." >&2
  echo "check-loopback-registries: a loopback registry here binds without the reverse DNS" >&2
  echo "check-loopback-registries: lookup http.server does on its own account, and writes" >&2
  echo "check-loopback-registries: its port to a file of its own. Keep the server_bind" >&2
  echo "check-loopback-registries: override in scripts/loopback-crate-registry.py and" >&2
  echo "check-loopback-registries: scripts/loopback-npm-registry.py, and keep both checks" >&2
  echo "check-loopback-registries: launching them. Then rerun 'just script-check'." >&2
  exit 1
fi
