#!/usr/bin/env bash
# Prove scripts/check-crate-sibling-resolution.sh reaches the same verdict when an index file
# it edits lands in the same whole second as the response cargo already cached.
#
# That is the condition its "sibling absent from the registry" case met by chance: the case
# rewrites the plugin-api index cargo cached while resolving the cases before it, and a
# registry that answers `If-Modified-Since` from a whole-second mtime said 304 Not Modified,
# so cargo kept the release the edit removed and the case passed or failed by the clock.
# scripts/loopback-crate-registry.py sends no validator and honours none for that reason.
#
# Left to the clock the condition arrives about half the time, so this forces it: the real
# check runs, with its real registry, through a python whose http.server reads every file's
# mtime as one fixed second. Only the registry's interpreter is touched — the shim is inert
# in any other python3 the check starts — and the shim records each mtime it pins, because a
# shim that did not take would make this pass for a registry that still answers 304.
#
# The whole check only proves the two halves together — cargo sends no conditional request
# when it was given no validator — so the registry is first asked directly: a plain request
# must carry no Last-Modified, and one carrying If-Modified-Since must still get the file.
set -euo pipefail

fatal() {
  echo "check-crate-sibling-resolution-same-second: $1" >&2
  echo "check-crate-sibling-resolution-same-second: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just distribution-check' does"
readonly ROOT
for tool in cargo git python3; do
  command -v "$tool" >/dev/null 2>&1 || fatal "$tool is not on PATH" "run 'just bootstrap', then rerun"
done

scratch="$(mktemp -d)" || fatal "could not create a scratch directory" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
server_pid=
cleanup() {
  if [ -n "$server_pid" ]; then kill "$server_pid" 2>/dev/null || true; fi
  rm -rf "$scratch"
}
trap cleanup EXIT
mkdir -p "$scratch/shim" "$scratch/index" || fatal "could not create $scratch's directories" \
  "check \$TMPDIR permissions, then rerun"
failures=0

# 1. The registry, asked directly for a file it serves.
printf 'the index as last written\n' > "$scratch/index/entry" || fatal \
  "could not write the file the registry is asked for" "check \$TMPDIR permissions, then rerun"
python3 "$ROOT/scripts/loopback-crate-registry.py" "$scratch/port" "$scratch/index" > "$scratch/server.log" 2>&1 &
server_pid=$!
for _ in $(seq 1 300); do
  [ -s "$scratch/port" ] && break
  kill -0 "$server_pid" 2>/dev/null || break
  sleep 0.1
done
[ -s "$scratch/port" ] || fatal \
  "scripts/loopback-crate-registry.py reported no port within 30s. It said:
$(sed 's/^/    /' "$scratch/server.log")" \
  "run 'bash scripts/check-loopback-registries.sh', which drives that launcher's start-up, and repair what it names"
if ! answer="$(PORT="$(cat "$scratch/port")" python3 - <<'PY' 2>&1
import os
import urllib.error
import urllib.request

url = f"http://127.0.0.1:{os.environ['PORT']}/entry"
want = b"the index as last written\n"
with urllib.request.urlopen(url, timeout=10) as plain:
    if plain.headers.get("Last-Modified") is not None:
        raise SystemExit(f"a plain request was answered with Last-Modified: {plain.headers['Last-Modified']}")
request = urllib.request.Request(url, headers={"If-Modified-Since": "Fri, 31 Dec 9999 23:59:59 GMT"})
try:
    with urllib.request.urlopen(request, timeout=10) as conditional:
        if conditional.status != 200 or conditional.read() != want:
            raise SystemExit(f"a conditional request was answered {conditional.status} without the file as written")
except urllib.error.HTTPError as refused:
    raise SystemExit(f"a request carrying If-Modified-Since was answered {refused.code} rather than with the file")
PY
)"; then
  echo "check-crate-sibling-resolution-same-second: FAILED (the registry asked directly): $answer" >&2
  echo "check-crate-sibling-resolution-same-second: next: keep scripts/loopback-crate-registry.py sending no Last-Modified and ignoring If-Modified-Since, then rerun" >&2
  failures=$((failures + 1))
fi
kill "$server_pid" 2>/dev/null || true
server_pid=

cat > "$scratch/shim/sitecustomize.py" <<'PY' || fatal \
  "could not write the mtime shim in $scratch, so nothing would be forced" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
"""http.server reading every file's mtime as one second, in the crate registry alone."""

import os
import sys

if sys.argv and os.path.basename(sys.argv[0]) == "loopback-crate-registry.py":
    import http.server

    PINNED = 1_700_000_000
    record = os.environ["ONETASKGRAPH_PINNED_MTIME_RECORD"]

    class _PinnedOs:
        def __getattr__(self, name):
            return getattr(os, name)

        def fstat(self, fd):
            real = os.fstat(fd)
            with open(record, "a", encoding="utf-8") as handle:
                handle.write("pinned\n")
            return os.stat_result(tuple(real[:7]) + (PINNED, PINNED, PINNED))

    http.server.os = _PinnedOs()
PY

# 2. The real check, with every file its registry serves read as modified in one second.
#    PYTHONPATH is this shim alone, so nothing the caller's environment names is imported.
record="$scratch/pinned"
if output="$(PYTHONPATH="$scratch/shim" \
  ONETASKGRAPH_PINNED_MTIME_RECORD="$record" \
  bash "$ROOT/scripts/check-crate-sibling-resolution.sh" 2>&1)"; then
  status=0
else
  status=$?
fi

[ -s "$record" ] || fatal \
  "the loopback registry served no file through the pinned mtime, so the same-second condition was never forced and this run proves nothing. The check said:
$(printf '%s\n' "$output" | sed 's/^/    /')" \
  "confirm scripts/check-crate-sibling-resolution.sh still launches scripts/loopback-crate-registry.py through python3 and that it still serves through http.server, then rerun"

if [ "$status" -ne 0 ]; then
  echo "check-crate-sibling-resolution-same-second: FAILED: with every index file's mtime in one second, scripts/check-crate-sibling-resolution.sh exited $status. It said:" >&2
  printf '%s\n' "$output" | sed 's/^/    /' >&2
  echo "check-crate-sibling-resolution-same-second: next: if a case above passed where a refusal was expected, the registry answered cargo's revalidation with 304 — keep scripts/loopback-crate-registry.py sending no Last-Modified and ignoring If-Modified-Since, then rerun" >&2
  failures=$((failures + 1))
fi
[ "$failures" -eq 0 ] || exit 1
