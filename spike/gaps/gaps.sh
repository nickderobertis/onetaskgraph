#!/usr/bin/env bash
# One measurement of one scenario of the board-copy-gaps feasibility spike:
#
#   spike/gaps/gaps.sh <scenario> [--build stock|head|closure]
#
# Scenarios and exit statuses are in spike/gaps/gaps/cli.py. The harness runs in the root uv
# workspace's environment, of which spike/gaps is a member. The live probes need the test board's
# credential, GH_PROJECTS_TOKEN; when it is unset it is read from SPIKE_TOKEN_ENV (default:
# ai-orchestrator's .env on this host).
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if ! command -v uv >/dev/null 2>&1; then
    echo "gaps.sh: uv is not on PATH; install it (https://docs.astral.sh/uv/) and run again" >&2
    exit 2
fi
token_env="${SPIKE_TOKEN_ENV:-$HOME/ai-orchestrator/.env}"
if [ -z "${GH_PROJECTS_TOKEN:-}" ] && [ -f "$token_env" ]; then
    GH_PROJECTS_TOKEN="$(sed -n 's/^GH_PROJECTS_TOKEN=//p' "$token_env" | head -n 1 | tr -d "\"'")"
    export GH_PROJECTS_TOKEN
fi
export PYTHONPATH="$here:$here/../harness"
exec uv run --quiet --frozen --directory "$here" python -m gaps "$@"
