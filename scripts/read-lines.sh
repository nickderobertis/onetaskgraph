#!/usr/bin/env bash
# `mapfile -t` for bash 3.2, which does not have that builtin. macos-latest ships 3.2, so
# scripts/check-bash4-array-builtins.sh refuses it and this is what to write instead.
#
#   source "$ROOT/scripts/read-lines.sh"
#   read_lines packages < "$packages_file"
#   read_lines crates < <(some | pipeline)
#
# The array is `mapfile -t`'s byte for byte: one element per line, spaces and backslashes
# intact, a blank line kept, a final line without a newline kept, and the carriage return
# left on — the two callers whose producer writes CRLF on Windows strip it themselves with
# `tr -d '\r'`, and stripping it here would change what the other two read.

# read_lines <array-name>, reading stdin.
read_lines() {
  # The character set is written out rather than as ranges or `[:alnum:]`: both are
  # locale-dependent inside a glob, and either would admit a non-ASCII name that is no
  # shell identifier at all — which is the one thing that must not reach the `eval` below.
  case "${1:-}" in
    "" | [0123456789]* | *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_]*)
      echo "read-lines: '${1:-}' is not a shell variable name, so it cannot be the array" >&2
      echo "read-lines: read_lines fills. Pass a bare name: read_lines packages < file" >&2
      return 1
      ;;
  esac
  # `eval` because bash 3.2 has no `declare -n`. The name checked above is the only thing
  # substituted into the evaluated text; the line is referenced there and expanded quoted,
  # so no byte of the input is ever evaluated as shell.
  local __read_lines_array="$1" __read_lines_line=""
  eval "$__read_lines_array=()"
  while IFS= read -r __read_lines_line || [ -n "$__read_lines_line" ]; do
    eval "$__read_lines_array+=(\"\$__read_lines_line\")"
  done
}
