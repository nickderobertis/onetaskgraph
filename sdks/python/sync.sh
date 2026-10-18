#!/usr/bin/env bash
# The Python SDK's environment, as sdk-python:bootstrap makes it.
#
#   [ONETASKGRAPH_SDK_WITHOUT_CLI=1] bash sdks/python/sync.sh [uv sync options]
#
# Ordinarily the whole lock, the onetaskgraph-cli workspace member included, which `uv sync`
# builds from this tree with maturin so a contributor's `uv run onetaskgraph` runs it. CI's
# pull-request jobs set ONETASKGRAPH_SDK_WITHOUT_CLI=1 and leave that copy out, with
# UV_NO_SYNC=1 so no later `uv run` installs it after all: nothing they run spawns it — the
# SDK's tests and every other spawner run onetaskgraph:build's, and the distribution journey
# installs the SDK beside the CLI wheel it builds — and building it is a maturin build of the
# whole binary on every runner. Options are passed to `uv sync`;
# tests/test_sync.py drives both modes.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")"
case "${ONETASKGRAPH_SDK_WITHOUT_CLI:-}" in
  1) exec uv sync --frozen --no-install-package onetaskgraph-cli "$@" ;;
  "") exec uv sync --frozen "$@" ;;
  *)
    echo "sdk-python-sync: ONETASKGRAPH_SDK_WITHOUT_CLI is '$ONETASKGRAPH_SDK_WITHOUT_CLI'; it is 1 or unset." >&2
    echo "sdk-python-sync: next: unset it to install the CLI too, or set it to 1 to leave it out." >&2
    exit 2
    ;;
esac
