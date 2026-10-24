#!/usr/bin/env bash
# One measurement of one scenario of the board-copy-gaps feasibility spike:
#
#   spike/gaps/gaps.sh <scenario> [--build stock|head|closure]
#
# Scenarios and exit statuses are in spike/gaps/gaps/cli.py. The live probes need the test
# board's credential, GH_PROJECTS_TOKEN; when it is unset it is read from SPIKE_TOKEN_ENV
# (default: ai-orchestrator's .env on this host).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if ! python="$(command -v python3)"; then
    echo "gaps.sh: python3 is not on PATH; install Python 3.13 or later and run again" >&2
    exit 2
fi
case "$python" in /*) ;; *) python="$(cd "$(dirname "$python")" && pwd)/python3" ;; esac
token_env="${SPIKE_TOKEN_ENV:-$HOME/ai-orchestrator/.env}"
if [ -z "${GH_PROJECTS_TOKEN:-}" ] && [ -f "$token_env" ]; then
    GH_PROJECTS_TOKEN="$(sed -n 's/^GH_PROJECTS_TOKEN=//p' "$token_env" | head -n 1 | tr -d "\"'")"
    export GH_PROJECTS_TOKEN
fi
export PYTHONPATH="$here:$here/../harness"
exec "$python" -m gaps "$@"
