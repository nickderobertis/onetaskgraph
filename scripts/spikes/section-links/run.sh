#!/usr/bin/env bash
# The section-links spike's one command: build every demonstration, then observe every link
# a browser can reach, then write the report's answers. Prints each item's URL or path; writes
# manifest.json, observations.json and answers.json into the output directory (default: $ONEPIPELINE_NODE_SCRATCH_DIR, else a
# fresh temporary directory), never into the repository.
#
#   scripts/spikes/section-links/run.sh [--out DIR]
#
# Needs: gh (authenticated for nickderobertis/onetaskgraph-live-scratch), GH_PROJECTS_TOKEN,
# GH_PROJECTS_OWNER, GH_PROJECTS_NUMBER, LINEAR_API_KEY, LINEAR_WRITE_TEAM=TES2, node and npm,
# uv, a Playwright Chromium under ~/.cache/ms-playwright, and the installed onetaskgraph CLI
# (ONETASKGRAPH names it; default: onetaskgraph on PATH).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
out="${ONEPIPELINE_NODE_SCRATCH_DIR:-}"
if [ "${1:-}" = "--out" ]; then out="$2"; fi
if [ -z "$out" ]; then out="$(mktemp -d)"; fi
out="$out/section-links-$(date -u +%Y%m%d-%H%M%S)"
mkdir -p "$out"

missing=""
for var in GH_PROJECTS_TOKEN GH_PROJECTS_OWNER GH_PROJECTS_NUMBER LINEAR_API_KEY LINEAR_WRITE_TEAM; do
  if [ -z "${!var:-}" ]; then missing="$missing $var"; fi
done
if [ -n "$missing" ]; then
  echo "missing:$missing — set them (LINEAR_WRITE_TEAM must be TES2) and run again" >&2
  exit 2
fi

python3 "$here/harness.py" all --out "$out"
uv run --quiet --no-project --with playwright==1.64.1 python "$here/observe.py" --out "$out"
python3 "$here/report.py" --out "$out"
echo "output: $out"
