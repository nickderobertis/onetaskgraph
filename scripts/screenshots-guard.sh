#!/usr/bin/env bash
# The local half of the STRICT screenshot gate, run by .githooks/pre-push after the gate.
#
# .github/workflows/visual-docs.yml fails on any capture that drifts from the committed
# baseline. This is what lets you regenerate and commit that baseline BEFORE pushing, so
# the workflow stays green: it re-captures only when a file that can change a rendered
# shot is in what is being pushed ([guard].paths in screencomp.toml), and on drift it
# refreshes the baseline, builds a review gallery and BLOCKS the push so the new bytes are
# committed deliberately.
#
# It reads the ref records git feeds a pre-push hook on STDIN — the hook hands on what it
# read — and does nothing at all when there are none: no records is nothing being pushed,
# which is also what keeps scripts/check-pre-push-provisioning.sh, which drives the real
# hook with an empty stdin, from paying for a capture it is not about.
# SCREENCOMP_GUARD_RANGE names a range directly instead, for the checks that drive this.
#
# It never prints `onevcs: host-prerequisite:`. That marker is the pinned release-plz's
# alone (AGENTS.md; scripts/check-pre-push-provisioning.sh holds the hook to exactly one
# line of it when that tool is missing and to none when it is there) — it tells the engine
# on this host to stop retrying, and a missing screenshot renderer is not that: the
# workflow still gates the capture, so this warns loudly and lets the push through.
#
# Usage: scripts/screenshots-guard.sh < <the pre-push ref records>
# Exit codes: 0 when there is nothing to check or the capture is unchanged; 1 on drift
# (the push is blocked) or when a capture this guard needed could not be made.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# No-op under CI: .github/workflows/visual-docs.yml is the source of truth there, and the
# gate job has no screencomp, no renderer and no business capturing.
[ -n "${CI:-}" ] && exit 0

# The one [capture].arches lane, read from screencomp.toml — the same single declaration
# scripts/screenshots.sh reads. The guard classifies that lane on every host, which is
# what byte-identity across machines buys; a second lane would need its own baseline and
# its own CI job, so refuse loudly rather than classifying against the wrong one.
LANE="$(sed -n 's/^arches *= *\[ *"\([^"]*\)" *\].*/\1/p' screencomp.toml)"
if ! [[ "$LANE" =~ ^[A-Za-z0-9_]+$ ]]; then
  echo "pre-push: expected exactly one lane in [capture].arches of screencomp.toml" >&2
  echo "pre-push: next: restore the single lane there, or give a second lane its own baseline and CI job" >&2
  exit 1
fi
readonly LANE
readonly MANIFEST="shots/baseline/$LANE.json"
readonly CURRENT="shots/current"
readonly GALLERY="shots/review"

# --- 1. What is being pushed --------------------------------------------------
ranges=()
if [ -n "${SCREENCOMP_GUARD_RANGE:-}" ]; then
  ranges+=("$SCREENCOMP_GUARD_RANGE")
else
  zero='^0+$'
  while read -r _local_ref local_sha _remote_ref remote_sha; do
    [ -z "${local_sha:-}" ] && continue
    if [[ "$local_sha" =~ $zero ]]; then
      continue # a branch being deleted pushes nothing to capture
    elif [[ "$remote_sha" =~ $zero ]]; then
      base="$(git merge-base origin/HEAD "$local_sha" 2>/dev/null || true)"
      if [ -n "$base" ]; then ranges+=("${base}..${local_sha}"); else ranges+=("$local_sha"); fi
    else
      ranges+=("${remote_sha}..${local_sha}")
    fi
  done
fi
[ "${#ranges[@]}" -eq 0 ] && exit 0

changed=""
for range in "${ranges[@]}"; do
  changed+="$(git diff --name-only "$range")"$'\n'
done
changed="$(printf '%s' "$changed" | sort -u)"

# --- 2. Without the CLI the guard cannot evaluate the push --------------------
# So it does not skip silently: it says what is missing and how to get it, and
# SCREENCOMP_GUARD_REQUIRE=1 turns the skip into a refusal for a machine that wants one.
if ! command -v screencomp >/dev/null 2>&1; then
  {
    echo "pre-push: screencomp is NOT on PATH, so the screenshot guard cannot evaluate this push."
    echo "pre-push: install it: https://github.com/nickderobertis/screencomp#install"
    echo "pre-push: the visual-docs workflow still gates the capture; set SCREENCOMP_GUARD_REQUIRE=1"
    echo "pre-push: to refuse the push here instead."
  } >&2
  [ -n "${SCREENCOMP_GUARD_REQUIRE:-}" ] && exit 1
  exit 0
fi

# --- 3. The cheap question: is anything screenshot-relevant in it? ------------
# `screencomp scope` exits 3 when a changed path matches [guard].paths, 0 when none does,
# and anything else on error. Only 3 is relevance; on an error warn and let the push go,
# because the workflow is the backstop and a guessed capture costs minutes.
set +e
printf '%s\n' "$changed" | screencomp scope --changed-from - --exit-code --quiet
scope_status=$?
set -e
case "$scope_status" in
  0) exit 0 ;;
  3) : ;;
  *)
    echo "pre-push: 'screencomp scope' failed (exit $scope_status), so the screenshot guard is skipped." >&2
    echo "pre-push: next: check that screencomp is current; the visual-docs workflow still gates this." >&2
    exit 0
    ;;
esac

# --- 4. Re-capture, natively: the shots are byte-identical on every machine ---
echo "pre-push: a file that can change a screenshot is in this push — re-capturing" >&2
if ! SHOTS_OUT="$CURRENT/$LANE" bash scripts/screenshots.sh; then
  echo "pre-push: the screenshot capture failed, so this push cannot be evaluated against" >&2
  echo "pre-push: $MANIFEST. Read the diagnostic above; 'just screenshots-tools' provisions" >&2
  echo "pre-push: the pinned renderer. Bypass deliberately with: git push --no-verify" >&2
  exit 1
fi

# --- 5. Classify the capture against the committed baseline -------------------
set +e
screencomp classify --baseline-manifest "$MANIFEST" --current "$CURRENT" --arch "$LANE" --exit-code
status=$?
set -e

if [ "$status" -eq 0 ]; then
  echo "pre-push: screenshots unchanged against $MANIFEST — ok to push" >&2
  exit 0
elif [ "$status" -ne 3 ]; then
  echo "pre-push: 'screencomp classify' failed (exit $status), so the capture was not evaluated." >&2
  echo "pre-push: next: read its diagnostic above and re-run 'git push'." >&2
  exit "$status"
fi

# --- On drift: regenerate the baseline, build a gallery, BLOCK the push -------
screencomp manifest --input "$CURRENT" --arch "$LANE" --output "$MANIFEST"
screencomp gallery --input "$CURRENT" --arch "$LANE" \
  --output "$GALLERY" --title "Pre-push screenshot review" >/dev/null

{
  echo
  echo "  SCREENSHOTS CHANGED — push blocked for review"
  echo "  Review the rendered gallery: $GALLERY/index.html"
  echo "  The refreshed baseline and README images are already written."
  echo "  If intended: git add $MANIFEST docs/screenshots && git commit, then push again."
  echo "  If not: investigate the diff. Bypass deliberately with: git push --no-verify"
  echo
} >&2
exit 1
