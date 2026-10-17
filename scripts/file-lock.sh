#!/usr/bin/env bash
# One holder at a time across processes, on bash 3.2 and on macOS, which ships no `flock`.
#
#   source "$ROOT/scripts/file-lock.sh"
#   with_file_lock "$ROOT/target/.release-build.lock" some-program and its arguments
#
# The lock is the kernel's, through perl's flock, which every runner this repository uses
# ships: the program runs holding it, and so does every process it starts, and the kernel
# releases it when the last of them ends — so a holder killed mid-build leaves nothing to break,
# and a build its holder started keeps it until that build is done.

# with_file_lock <lock-file> <program...>: run the program holding the lock; the status is the
# program's, or 1 with the reason when the lock cannot be taken at all.
with_file_lock() {
  local lock="$1"
  shift
  if ! command -v perl >/dev/null 2>&1; then
    echo "file-lock: perl is not on PATH, and it is what takes the lock on $lock." >&2
    echo "file-lock: next: install perl (every hosted runner ships it), then rerun." >&2
    return 1
  fi
  mkdir -p "$(dirname "$lock")" || {
    echo "file-lock: could not make the directory of $lock; next: make it writable, then rerun." >&2
    return 1
  }
  # $^F above every descriptor keeps the lock's open across exec, into the program.
  perl -MFcntl=:flock -e '
    $^F = 1024;
    my $lock = shift;
    open(my $handle, ">>", $lock) or die "file-lock: could not open $lock: $!; next: make it writable, then rerun.\n";
    flock($handle, LOCK_EX) or die "file-lock: could not lock $lock: $!; next: check the filesystem supports locks, then rerun.\n";
    exec { $ARGV[0] } @ARGV or die "file-lock: could not run $ARGV[0]: $!; next: install it or put it on PATH, then rerun.\n";
  ' "$lock" "$@"
}
