#!/usr/bin/env bash
# One measurement of one named scenario (see spike/README.md):
#
#   spike/copy-timing.sh <scenario> [--build stock|span|head|closure]
#   spike/copy-timing.sh provision      # clone ai-orchestrator and build the span build
#
# Runs and state go under ${XDG_CACHE_HOME:-$HOME/.cache}/$SPIKE_CACHE_NAME, by default the
# feasibility spike's board-copy-gaps-2026-10-24 (the timing spike's was
# board-copy-speed-2026-10-18). The offline scenarios (reach-*, screen-union, proxy-refusal)
# need no credential.
#
# The test board's credential is GH_PROJECTS_TOKEN; when it is unset it is read from
# SPIKE_TOKEN_ENV (default: ai-orchestrator's .env on this host).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export SPIKE_CACHE_NAME="${SPIKE_CACHE_NAME:-board-copy-gaps-2026-10-24}"
cache="${XDG_CACHE_HOME:-$HOME/.cache}/$SPIKE_CACHE_NAME"
python="$(command -v python3)"
case "$python" in /*) ;; *) python="$(cd "$(dirname "$python")" && pwd)/python3" ;; esac
if [ "${1:-}" = "provision" ]; then
    mkdir -p "$cache"
    if [ ! -d "$cache/ai-orchestrator" ]; then
        git clone -q https://github.com/nickderobertis/ai-orchestrator.git "$cache/ai-orchestrator"
    fi
    (cd "$cache/ai-orchestrator" && uv sync -q --frozen)
    "$here/span-build/build.sh"
    exit 0
fi
token_env="${SPIKE_TOKEN_ENV:-$HOME/ai-orchestrator/.env}"
if [ -z "${GH_PROJECTS_TOKEN:-}" ] && [ -f "$token_env" ]; then
    GH_PROJECTS_TOKEN="$(sed -n 's/^GH_PROJECTS_TOKEN=//p' "$token_env" | head -n 1 | tr -d "\"'")"
    export GH_PROJECTS_TOKEN
fi
exec "$python" "$here/harness/run.py" "$@"
