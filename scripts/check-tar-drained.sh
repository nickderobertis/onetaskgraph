#!/usr/bin/env bash
# Refuse a tar extraction from a pipe anywhere under scripts/ but extract_tar_stream.
#
# bsdtar — macOS's tar — stops reading at the end-of-archive marker and exits with the
# record padding after it still unread. The producer's last write then lands on a closed
# pipe, and on a hosted runner, where SIGPIPE is ignored, it fails `tar: Write error` and
# fails the copy under pipefail, by timing alone.
#
# Write `copy_tracked_files` or `extract_tar_stream` from scripts/scratch-clone.sh instead:
# the second drains what tar leaves, and the first copies through it.
#
# Usage: check-tar-drained.sh [<root>] — <root> defaults to this repository, and is what
# scripts/check-tar-drained-enforced.sh points at a planted copy.
set -euo pipefail

fatal() {
  echo "check-tar-drained: $1" >&2
  echo "check-tar-drained: next: $2" >&2
  exit 1
}

ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just script-check' does"
readonly ROOT
[ -d "$ROOT/scripts" ] || fatal \
  "$ROOT has no scripts/ directory to scan" \
  "run the check from a checkout of this repository, or name one as its argument"

# An extraction that reads standard input: `x` among tar's short flags, or `--extract`,
# with `-f -` or nothing after it but a pipe could feed. A full-line comment is skipped;
# past that this is a line scan, conservative in the safe direction. The one permitted site
# is the drain itself, matched by the whole line so a second extraction in that file is
# still refused; its pattern spells `[t]ar` so that it is not a match of its own.
readonly EXTRACT='(^|[[:space:];&|(])tar[[:space:]]+(-?[A-Za-z]*x[A-Za-z]*|--extract)([[:space:]]|$)'
readonly FROM_STDIN='(-[A-Za-z]*f[[:space:]]+-([[:space:]]|$)|--file[= ]-([[:space:]]|$)|\|[[:space:]]*tar[[:space:]])'
# shellcheck disable=SC2016 # the \$1 is the helper's own source text, matched literally
readonly DRAIN='^  [t]ar -xf - -C "\$1" && cat >/dev/null && return 0$'

# grep exits 1 for no match and 2 for a scan it could not make; only the first is a pass.
# It scans from inside the root, so every hit starts `scripts/` whatever the root is: a
# Windows root can carry a drive letter's colon, which splitting the hit on its first two
# colons would otherwise take for the file's end and refuse the drain itself.
status=0
hits="$(cd "$ROOT" && grep -rnE -- "$EXTRACT" scripts)" || status=$?
[ "$status" -le 1 ] || fatal \
  "grep could not scan $ROOT/scripts (exit $status), so no script under it is cleared" \
  "fix what grep reported above — usually an unreadable file under scripts/ — then rerun"

found=""
while IFS= read -r hit; do
  [ -n "$hit" ] || continue
  line="${hit#*:*:}"
  # A carriage return a CRLF checkout leaves is not part of the line the drain is matched by.
  line="${line%$'\r'}"
  trimmed="${line#"${line%%[![:space:]]*}"}"
  case "$trimmed" in
    '#'*) continue ;;
  esac
  [[ $line =~ $FROM_STDIN ]] || continue
  [ "${hit%%:*}" = scripts/scratch-clone.sh ] && [[ $line =~ $DRAIN ]] && continue
  found="$found$hit"$'\n'
done <<EOF
$hits
EOF

if [ -n "$found" ]; then
  printf '%s' "$found" >&2
  fatal "these lines extract a tar stream from a pipe without draining it, which fails at random on macOS" \
    "write 'copy_tracked_files <root> <dest>' or '<producer> | extract_tar_stream <dest>' from scripts/scratch-clone.sh instead"
fi
