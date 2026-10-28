#!/usr/bin/env bash
# llmlint: ignore-file[new_code_lands_in_a_project] This is the measurement record beside the captures it produced: run by hand against GitHub through measure.sh, never by a target. docs/measurements is no Nx project (nor is coalesced-members-dry-run/dry_run.py beside it), and making it one would put a live-GitHub harness inside affected selection.
# llmlint: ignore-file[changed_behavior_has_e2e] Its proof is the live run itself, whose captures are committed and reconciled by test_analyze.py; the GitHub-facing parts can be driven only against GitHub's test board, which measure.sh does, and the sanitizer produces every capture those tests read.
# The board-copy-targets measurement on GitHub (docs/measurements/board-copy-targets-2026-10-26.md):
#
#   docs/measurements/board-copy-targets-2026-10-26/measure.sh provision   # the host clone, once
#   docs/measurements/board-copy-targets-2026-10-26/measure.sh all         # every row, then the analysis
#   docs/measurements/board-copy-targets-2026-10-26/measure.sh <scenario>  # one row (see harness/run.py)
#
# `all` takes the seven rows in order — bare-first, bare-changed1, bare-unchanged, host-first,
# host-changed1, host-unchanged, host-approve — prices the operations they sent, deletes every
# issue they created, and recomputes the report's tables with analyze.py. It writes only to the
# test board and its scratch repository, pausing while GraphQL holds under 40% of its allowance.
# It prints one line on success; every step's own output is kept in a log under the cache.
# The test board's credential is GH_PROJECTS_TOKEN; when it is unset it is read from
# SPIKE_TOKEN_ENV (default: ai-orchestrator's .env on this host). Raw logs stay under
# ${XDG_CACHE_HOME:-$HOME/.cache}/board-copy-targets-2026-10-26; sanitized ones land in captures/.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cache="${XDG_CACHE_HOME:-$HOME/.cache}/board-copy-targets-2026-10-26"
host_commit="${SPIKE_HOST_COMMIT:-668dc496}"
python="$(command -v python3)" || {
    echo "measure.sh: python3 is not on PATH; install it and run again" >&2
    exit 2
}
case "$python" in /*) ;; *) python="$(cd "$(dirname "$python")" && pwd)/python3" ;; esac
if [ "${1:-}" = "" ]; then
    echo "usage: $0 provision | all | <scenario>" >&2
    exit 2
fi
if [ "$1" = "provision" ]; then
    mkdir -p "$cache"
    if [ ! -d "$cache/ai-orchestrator" ]; then
        git clone -q https://github.com/nickderobertis/ai-orchestrator.git "$cache/ai-orchestrator"
    fi
    (cd "$cache/ai-orchestrator" && git checkout -q "$host_commit" && uv sync -q --frozen \
        && bun install --frozen-lockfile >/dev/null)
    exit 0
fi
token_env="${SPIKE_TOKEN_ENV:-$HOME/ai-orchestrator/.env}"
if [ -z "${GH_PROJECTS_TOKEN:-}" ] && [ -f "$token_env" ]; then
    GH_PROJECTS_TOKEN="$(sed -n 's/^GH_PROJECTS_TOKEN=//p' "$token_env" | head -n 1 | tr -d "\"'")"
    export GH_PROJECTS_TOKEN
fi
# Every step's own progress goes to one log; this prints one line, or the failing step's tail.
mkdir -p "$cache"
log="$cache/measure-$(date -u +%Y%m%dT%H%M%S).log"
quiet() {
    if ! "$@" >>"$log" 2>&1; then
        tail -n 30 "$log" >&2
        echo "measure.sh: '$*' failed; its whole output is in $log" >&2
        exit 1
    fi
}
if [ "$1" != "all" ]; then
    quiet "$python" "$here/harness/run.py" "$@"
    echo "measure.sh: $1 captured under $here/captures/ (log: $log)"
    exit 0
fi
for scenario in bare-first bare-changed1 bare-unchanged host-first host-changed1 \
    host-unchanged host-approve; do
    quiet "$python" "$here/harness/run.py" "$scenario"
done
quiet "$python" "$here/harness/pricing.py"
quiet "$python" "$here/harness/run.py" cleanup
"$python" "$here/analyze.py" --write
echo "measure.sh: seven rows captured, priced and cleaned up; the report's tables are recomputed (log: $log)"
