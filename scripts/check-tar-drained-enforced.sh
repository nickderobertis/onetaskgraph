#!/usr/bin/env bash
# Watch scripts/check-tar-drained.sh refuse, and watch the drain it sends every site to work.
#
# Three cases. The guard clears this tree. It refuses an undrained extraction planted in a
# copy of scripts/ in each spelling it reads — `-xf -`, a bare `-x` fed by a pipe, and one
# with a trailing comment — while passing over a commented-out one. And extract_tar_stream
# reads its stream to the end where a bare `tar -xf -` does not: a stream left unread is
# what a macOS producer writes into a closed pipe, so the drain's whole claim is that no
# byte is left. The stream is a regular file rather than a pipe so the bytes left over can
# be counted without a race, and because GNU tar drains a pipe of its own accord, which
# would make a pipe-fed case pass whether or not the helper drained.
set -euo pipefail

fatal() {
  echo "check-tar-drained-enforced: $1" >&2
  echo "check-tar-drained-enforced: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just script-check' does"
readonly ROOT
readonly GUARD="$ROOT/scripts/check-tar-drained.sh"

# The path is built from $ROOT at run time, so ShellCheck cannot follow it; the directive
# names the file it resolves to. Tested before it is sourced rather than guarded after:
# bash 3.2 ends the shell where `source` cannot find its file, so the handler after `||`
# never runs there and the reader is told nothing about what to restore.
# shellcheck source=scripts/scratch-clone.sh
if [ ! -r "$ROOT/scripts/scratch-clone.sh" ] || ! source "$ROOT/scripts/scratch-clone.sh"; then
  fatal "could not load $ROOT/scripts/scratch-clone.sh, whose drain is under test" \
    "restore it with 'git checkout -- scripts/scratch-clone.sh' and rerun"
fi

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree this check plants into" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

# Case 1: this tree is clean.
output="$(bash "$GUARD" 2>&1)" || fatal \
  "the guard refused this tree: $output" \
  "route each line it names through copy_tracked_files or extract_tar_stream in scripts/scratch-clone.sh"

# Case 2: every planted spelling is refused by file and line, and the comment is not.
mkdir -p "$scratch/plant" || fatal "could not create $scratch/plant" "check \$TMPDIR, then rerun"
cp -R "$ROOT/scripts" "$scratch/plant/scripts" || fatal \
  "could not copy scripts/ into $scratch/plant" "check 'df -h' for free space, then rerun"
# The extraction is assembled from a variable so this file does not hold one of its own.
x='tar -x'
cat >"$scratch/plant/scripts/planted.sh" <<EOF
#!/usr/bin/env bash
git archive HEAD | ${x}f - -C "\$1"
producer | ${x} -C "\$1"
producer | ${x}f - -C "\$1" # a trailing comment
  # producer | ${x}f - -C "\$1"
EOF
status=0
output="$(bash "$GUARD" "$scratch/plant" 2>&1)" || status=$?
[ "$status" -eq 1 ] || fatal \
  "the guard exited $status over a planted undrained extraction, not 1: $output" \
  "check that scripts/check-tar-drained.sh still scans every file under scripts/"
for line in 2 3 4; do
  printf '%s\n' "$output" | grep -qF "scripts/planted.sh:$line:" || fatal \
    "the guard did not name scripts/planted.sh:$line, an undrained extraction: $output" \
    "check the spellings scripts/check-tar-drained.sh reads as an extraction from a pipe"
done
if printf '%s\n' "$output" | grep -qF "scripts/planted.sh:5:"; then
  fatal "the guard refused scripts/planted.sh:5, which is a comment: $output" \
    "check how scripts/check-tar-drained.sh skips a full-line comment"
fi

# Case 3: the drain leaves no byte of its stream unread, where a bare extraction does.
mkdir -p "$scratch/src" "$scratch/bare" "$scratch/drained" || fatal \
  "could not create the extraction directories" "check \$TMPDIR, then rerun"
printf 'payload\n' >"$scratch/src/file.txt"
{ tar -cf - -C "$scratch/src" file.txt && head -c 1048576 /dev/zero; } >"$scratch/stream" || fatal \
  "could not build the archive stream with a trailer" "check that tar and head are on PATH, then rerun"
left() { tr -d ' \r' <"$1"; }
# The bare extraction is spelled from $x for the reason the planted ones are: the guard
# would read it as a site, and here its leaving the stream unread is the point.
# shellcheck disable=SC2086 # $x is split on purpose, into the command and its flag
( ${x}f - -C "$scratch/bare" && cat ) <"$scratch/stream" | wc -c >"$scratch/bare.left"
[ "$(left "$scratch/bare.left")" -gt 0 ] || fatal \
  "a bare 'tar -xf -' read its whole stream here, so this host cannot show what the drain is for" \
  "report this; the case needs a tar that stops at its end-of-archive marker, as GNU tar and bsdtar both do on a file"
( extract_tar_stream "$scratch/drained" && cat ) <"$scratch/stream" | wc -c >"$scratch/drained.left"
[ "$(left "$scratch/drained.left")" -eq 0 ] || fatal \
  "extract_tar_stream left $(left "$scratch/drained.left") bytes of its stream unread, which a macOS producer writes into a closed pipe" \
  "restore the 'cat >/dev/null' after the extraction in scripts/scratch-clone.sh"
cmp -s "$scratch/src/file.txt" "$scratch/drained/file.txt" || fatal \
  "extract_tar_stream drained its stream but did not extract file.txt intact" \
  "check the extraction in scripts/scratch-clone.sh"
