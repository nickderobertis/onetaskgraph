#!/usr/bin/env bash
# Refresh the committed digest baseline from the capture that is already on disk.
#
# Run after an INTENDED output change, through `just screenshots-bless`, which captures
# first. It reads the lane out of screencomp.toml — the one place it is declared — so the
# baseline it writes is the file .github/workflows/visual-docs.yml and .githooks/pre-push
# both classify against, and never a second spelling of that name.
#
# Commit shots/baseline/<lane>.json together with the refreshed docs/screenshots/, which
# the capture has already written.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

LANE="$(sed -n 's/^arches *= *\[ *"\([^"]*\)" *\].*/\1/p' screencomp.toml)"
if ! [[ "$LANE" =~ ^[A-Za-z0-9_]+$ ]]; then
  echo "screenshots-bless: expected exactly one lane in [capture].arches of screencomp.toml; got: [${LANE:-none}]" >&2
  echo "screenshots-bless: next: restore the single lane there, or give a second lane its own baseline and CI job" >&2
  exit 1
fi
readonly LANE

if [ ! -r "shots/current/$LANE/captures.json" ]; then
  echo "screenshots-bless: there is no capture at shots/current/$LANE to bless" >&2
  echo "screenshots-bless: next: run 'just screenshots' first — or 'just screenshots-bless', which does" >&2
  exit 1
fi

command -v screencomp >/dev/null 2>&1 || {
  echo "screenshots-bless: screencomp is not installed, so the baseline cannot be written" >&2
  echo "screenshots-bless: next: install it from https://github.com/nickderobertis/screencomp#install and re-run" >&2
  exit 1
}

mkdir -p shots/baseline || {
  echo "screenshots-bless: could not create shots/baseline, where the committed digest baseline lives" >&2
  echo "screenshots-bless: next: check the permissions of the shots directory, then re-run 'just screenshots-bless'" >&2
  exit 1
}
if ! screencomp manifest --input shots/current --arch "$LANE" \
  --output "shots/baseline/$LANE.json" --quiet; then
  echo "screenshots-bless: the baseline at shots/baseline/$LANE.json was not written" >&2
  echo "screenshots-bless: next: read the diagnostic above, then re-run 'just screenshots-bless'; the capture in shots/current/$LANE is still there" >&2
  exit 1
fi
echo "screenshots-bless: refreshed shots/baseline/$LANE.json; commit it with docs/screenshots/" >&2
