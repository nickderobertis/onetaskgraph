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

# git writes one record per ref, `<local ref> <local sha> <remote ref> <remote sha>`, and
# what arrives on this stdin is external input: it is whatever the caller piped. So each
# sha is held to the shape git writes AND resolved in this repository before it becomes
# half of a range — an unresolvable one would make `git diff` fail and read as the guard
# having broken rather than as a record it could not use.
resolves() {
  [[ "$1" =~ ^[0-9a-f]{40}$|^[0-9a-f]{64}$ ]] \
    && git rev-parse --verify --quiet "$1^{commit}" >/dev/null 2>&1
}

# Two kinds of answer to "what is this push adding": a range between two commits, and — for
# a branch nothing bounds — the whole of one commit's tree.
ranges=()
whole_trees=()
if [ -n "${SCREENCOMP_GUARD_RANGE:-}" ]; then
  # A range a caller chose, so it is validated before it reaches `git diff` exactly as the
  # records are — by resolution rather than by shape, because a person naming one types
  # `origin/main..HEAD` rather than two object ids, and as exactly TWO endpoints: a single
  # revision is a different `git diff`, against the working tree rather than between two
  # commits, which would answer a question this guard is not asking.
  before="${SCREENCOMP_GUARD_RANGE%%..*}"
  after="${SCREENCOMP_GUARD_RANGE##*..}"
  if [ "$before..$after" != "$SCREENCOMP_GUARD_RANGE" ] || [ -z "$before" ] || [ -z "$after" ]; then
    echo "pre-push: SCREENCOMP_GUARD_RANGE is '$SCREENCOMP_GUARD_RANGE', which is not two revisions joined by '..'" >&2
    echo "pre-push: next: name a range, as in 'origin/main..HEAD', or unset it to use what git is pushing" >&2
    exit 1
  fi
  for endpoint in "$before" "$after"; do
    git rev-parse --verify --quiet "$endpoint^{commit}" >/dev/null 2>&1 || {
      echo "pre-push: SCREENCOMP_GUARD_RANGE is '$SCREENCOMP_GUARD_RANGE', and '$endpoint' is not a commit this repository can resolve" >&2
      echo "pre-push: next: name two resolvable revisions, or unset it to use what git is pushing" >&2
      exit 1
    }
  done
  ranges+=("$SCREENCOMP_GUARD_RANGE")
else
  zero='^0+$'
  while read -r record; do
    [ -z "$record" ] && continue
    # Four fields, or it is not one of git's records: with three, the remote sha would read
    # as empty and the ref would be reclassified as a branch nothing bounds; with five, the
    # last field would absorb the rest. Either way the guard would be answering about
    # something other than what is being pushed.
    # shellcheck disable=SC2086
    set -- $record
    if [ "$#" -ne 4 ]; then
      echo "pre-push: the screenshot guard skipped a ref record with $# field(s) rather than the four git writes: '$record'" >&2
      echo "pre-push: next: nothing to do if the push succeeds; the visual-docs workflow gates the capture either way." >&2
      continue
    fi
    local_sha="$2"
    remote_sha="$4"
    if [[ "$local_sha" =~ $zero ]]; then
      continue # a branch being deleted pushes nothing to capture
    fi
    if ! resolves "$local_sha"; then
      echo "pre-push: the screenshot guard cannot read '$local_sha' as a commit of this repository, so it skipped this ref." >&2
      echo "pre-push: next: nothing to do if the push succeeds; the visual-docs workflow gates the capture either way." >&2
      continue
    fi
    if [[ "$remote_sha" =~ $zero ]] || ! resolves "$remote_sha"; then
      # A branch the remote has never seen, or one whose tip this clone cannot resolve:
      # what the push adds is what it added since it forked from the default branch.
      base="$(git merge-base origin/HEAD "$local_sha" 2>/dev/null || true)"
      if [ -n "$base" ]; then
        ranges+=("${base}..${local_sha}")
      else
        # Nothing bounds it — an orphan branch, or a clone with no default branch to fork
        # from — so every file the branch carries counts. `git diff` with ONE revision
        # would compare that commit against the working tree, which is a different question
        # and answers "nothing changed" for a push that adds everything; this guard fails
        # toward capturing, as every other selection decision in this repository does.
        whole_trees+=("$local_sha")
      fi
    else
      ranges+=("${remote_sha}..${local_sha}")
    fi
  done
fi
if [ "${#ranges[@]}" -eq 0 ] && [ "${#whole_trees[@]}" -eq 0 ]; then
  exit 0
fi

changed=""
for range in "${ranges[@]+"${ranges[@]}"}"; do
  changed+="$(git diff --name-only "$range")"$'\n'
done
for tree in "${whole_trees[@]+"${whole_trees[@]}"}"; do
  changed+="$(git ls-tree -r --name-only "$tree")"$'\n'
done
changed="$(printf '%s' "$changed" | sort -u)"

# So it does not skip silently: it says what is missing and how to get it, and
# SCREENCOMP_GUARD_REQUIRE=1 turns the skip into a refusal for a machine that wants one.
case "${SCREENCOMP_GUARD_REQUIRE:-}" in
  "" | 0 | false | no) required=0 ;;
  1 | true | yes) required=1 ;;
  *)
    echo "pre-push: SCREENCOMP_GUARD_REQUIRE is '$SCREENCOMP_GUARD_REQUIRE', which is neither true nor false, so whether a missing screencomp refuses this push is undecided" >&2
    echo "pre-push: next: set it to 1 or 0 (or unset it, which is 0), then push again" >&2
    exit 1
    ;;
esac
if ! command -v screencomp >/dev/null 2>&1; then
  echo "pre-push: screencomp is not on PATH, so the screenshot guard cannot evaluate this push — install it (https://github.com/nickderobertis/screencomp#install), or set SCREENCOMP_GUARD_REQUIRE=1 to refuse here instead; the visual-docs workflow still gates the capture." >&2
  [ "$required" -eq 1 ] && exit 1
  exit 0
fi

# `screencomp scope` exits 3 when a changed path matches [guard].paths, 0 when none does,
# and anything else on error. Only 3 is relevance; on an error warn and let the push go,
# because the workflow is the backstop and a guessed capture costs minutes.
# Through a file rather than a pipeline: under `pipefail` a `printf | screencomp` whose
# reader exits before draining reports printf's SIGPIPE as the pipeline's status, and this
# branches on that status. A redirection has one status, screencomp's own.
changed_list="$(mktemp)"
trap 'rm -f "$changed_list"' EXIT
printf '%s\n' "$changed" > "$changed_list"
set +e
screencomp scope --changed-from - --exit-code --quiet < "$changed_list"
scope_status=$?
set -e
case "$scope_status" in
  0) exit 0 ;;
  3) : ;;
  *)
    echo "pre-push: 'screencomp scope' failed (exit $scope_status), so the screenshot guard is skipped — check that screencomp is current; the visual-docs workflow still gates this." >&2
    exit 0
    ;;
esac

# No line of our own here: the capture below runs cargo and the renderer, whose own output
# is what says a multi-minute step is under way, and a push this guard lets through has
# nothing to report. Only a refusal speaks.
if ! SHOTS_OUT="$CURRENT/$LANE" bash scripts/screenshots.sh; then
  echo "pre-push: the screenshot capture failed, so this push cannot be evaluated against" >&2
  echo "pre-push: $MANIFEST. Read the diagnostic above; 'just screenshots-tools' provisions" >&2
  echo "pre-push: the pinned renderer. Bypass deliberately with: git push --no-verify" >&2
  exit 1
fi

set +e
screencomp classify --baseline-manifest "$MANIFEST" --current "$CURRENT" --arch "$LANE" --exit-code
status=$?
set -e

if [ "$status" -eq 0 ]; then
  # Quiet from here: the line above already said a capture was happening, and the push
  # going on is the outcome. Only a refusal has more to say.
  exit 0
elif [ "$status" -ne 3 ]; then
  echo "pre-push: 'screencomp classify' failed (exit $status), so this push was not evaluated" >&2
  echo "pre-push: against $MANIFEST." >&2
  echo "pre-push: next: read its diagnostic above, check that screencomp is current, then push again." >&2
  # 1 rather than screencomp's own status: this script's exit codes are its own contract,
  # and passing a third party's through would make an unknown number the hook's answer.
  exit 1
fi

# On drift the baseline and the gallery are regenerated before the push is refused, so
# that saying "yes, that is the new output" is one `git add` rather than a second run.
if ! screencomp manifest --input "$CURRENT" --arch "$LANE" --output "$MANIFEST"; then
  echo "pre-push: the capture drifted and the refreshed baseline could not be written to $MANIFEST." >&2
  echo "pre-push: next: read the diagnostic above, then run 'just screenshots-bless' by hand and commit it." >&2
  exit 1
fi
if ! screencomp gallery --input "$CURRENT" --arch "$LANE" \
  --output "$GALLERY" --title "Pre-push screenshot review" >/dev/null; then
  echo "pre-push: the capture drifted and the review gallery could not be built at $GALLERY." >&2
  echo "pre-push: next: read the diagnostic above; the refreshed baseline and the README images are written, so 'git diff' is the other way to review them." >&2
  exit 1
fi

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
