#!/usr/bin/env bash
# Refuse a tar extraction from a pipe anywhere under scripts/ but extract_tar_stream.
#
# bsdtar — macOS's tar — stops reading at the end-of-archive marker and exits with the
# record padding after it still unread. The producer's last write then lands on a closed
# pipe, and on a hosted runner, where SIGPIPE is ignored, it fails `tar: Write error` and
# fails the copy under pipefail — by timing alone, so `check (macos-latest)` refused a
# branch twice on two different guards that had never failed before.
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
readonly DRAIN='^  [t]ar -xf - -C "\$1" && cat >/dev/null$'

found=""
while IFS= read -r hit; do
  line="${hit#*:*:}"
  trimmed="${line#"${line%%[![:space:]]*}"}"
  case "$trimmed" in
    '#'*) continue ;;
  esac
  printf '%s\n' "$line" | grep -Eq -- "$FROM_STDIN" || continue
  [ "${hit%%:*}" = "$ROOT/scripts/scratch-clone.sh" ] && printf '%s\n' "$line" | grep -Eq -- "$DRAIN" && continue
  found="$found${hit#"$ROOT"/}"$'\n'
done <<EOF
$(grep -rnE -- "$EXTRACT" "$ROOT/scripts" 2>/dev/null || true)
EOF

if [ -n "$found" ]; then
  printf '%s' "$found" >&2
  fatal "these lines extract a tar stream from a pipe without draining it, which fails at random on macOS" \
    "write 'copy_tracked_files <root> <dest>' or '<producer> | extract_tar_stream <dest>' from scripts/scratch-clone.sh instead"
fi
