#!/usr/bin/env bash
# One holder at a time across processes, for bash 3.2, which has no `flock` on macOS.
#
#   source "$ROOT/scripts/directory-lock.sh"
#   with_directory_lock "$ROOT/target/.release-build.lock" some command and its arguments
#
# The lock is a directory, because `mkdir` either makes it or fails, atomically, on every
# platform this repository runs on. It names its holder's process id, so a lock whose holder
# has died — a run killed mid-build — is taken over rather than waited on for ever; one that
# names no holder at all (killed between making it and writing the id) is taken over once it
# is a minute old.

# with_directory_lock <lock> <command...>: run the command holding the lock, and release it
# however the command ends; the status is the command's.
with_directory_lock() {
  local lock="$1" holder status
  shift
  mkdir -p "$(dirname "$lock")"
  until mkdir "$lock" 2>/dev/null; do
    holder="$(cat "$lock/pid" 2>/dev/null || true)"
    if [ -n "$holder" ]; then
      kill -0 "$holder" 2>/dev/null || rm -rf "$lock"
    elif [ -n "$(find "$lock" -maxdepth 0 -mmin +1 2>/dev/null)" ]; then
      rm -rf "$lock"
    fi
    sleep 1
  done
  echo "$$" >"$lock/pid"
  if "$@"; then status=0; else status=$?; fi
  rm -rf "$lock"
  return "$status"
}
