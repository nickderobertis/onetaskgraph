# Sourced by the probe interpreter scripts/check-relative-interpreter.sh plants, before it
# runs the real python3: records which call site reached it.
#
#   . scripts/python-call-site.sh <probe directory> <the arguments python3 was given...>
#
# A call site is the script that started the interpreter and the program it gave it, one
# tab-separated line appended to <probe directory>/call-sites.<run>, where <run> is what
# <probe directory>/current names. The script is the first `.sh` argument of the parent
# process, from its last `scripts/` on, or `bash -c <digest>` for an inline body such as a
# release workflow step; the program is the script path from its last `scripts/` on (or its
# name), `-m <module>`, `-W`/`-X` without its argument, or `-c <digest>` or `stdin <digest>` of the code itself, which is read
# here and handed on to the interpreter unchanged. A site it cannot name is recorded as
# `unknown`; an unwritable record refuses the call, because the comparison needs its evidence.
python_call_site_record() {
  local probe="$1" run caller="" program="" argument digest
  recording_failed() {
    echo "python-call-site: could not $1" >&2
    echo "python-call-site: next: check free space and write permissions in $probe, then rerun the relative-interpreter check" >&2
    exit 1
  }
  shift
  run="$(cat "$probe/current" 2>/dev/null)" || run=none
  [ -n "$run" ] || run=none
  digest_of() { cksum | cut -d' ' -f1; }
  # The parent's arguments: /proc on Linux and in Git for Windows' MSYS, ps on macOS.
  local -a parent=()
  if [ -r "/proc/$PPID/cmdline" ]; then
    while IFS= read -r -d '' argument; do parent+=("$argument"); done <"/proc/$PPID/cmdline"
  else
    read -r -a parent <<<"$(ps -o args= -p "$PPID" 2>/dev/null)"
  fi
  local index=0 count="${#parent[@]}"
  while [ "$index" -lt "$count" ]; do
    argument="${parent[$index]}"
    case "$argument" in
      *.sh) caller="scripts/${argument##*scripts/}"; [ "$argument" != "${argument##*scripts/}" ] || caller="$(basename "$argument")"; break ;;
      -c)
        if [ $((index + 1)) -lt "$count" ]; then
          caller="bash -c $(printf '%s' "${parent[$((index + 1))]}" | digest_of)"
          break
        fi
        ;;
    esac
    index=$((index + 1))
  done
  [ -n "$caller" ] || caller=unknown
  while [ "$#" -gt 0 ]; do
    case "$1" in
      -c) program="-c $(printf '%s' "${2:-}" | digest_of)"; break ;;
      -m) program="-m ${2:-}"; break ;;
      # Each takes the next argument; one given none is recorded as such and left for the
      # interpreter to refuse, rather than looping on a shift that cannot consume it.
      -W | -X)
        if [ "$#" -lt 2 ]; then
          program="$1 without its argument"
          break
        fi
        shift 2
        ;;
      -) break ;;
      -*) shift ;;
      *) program="${1##*scripts/}"; [ "$1" = "$program" ] && program="$(basename "$1")" || program="scripts/$program"; break ;;
    esac
  done
  if [ -z "$program" ]; then
    # Recording must leave stdin intact for the real interpreter that runs next.
    local code="$probe/stdin.$$"
    cat >"$code" || recording_failed "save the Python input to $code"
    program="stdin $(digest_of <"$code")"
    exec 0<"$code" || recording_failed "reopen the Python input from $code"
    rm -f "$code" || recording_failed "remove the saved Python input $code"
  fi
  printf '%s\t%s\n' "$caller" "$program" >>"$probe/call-sites.$run" \
    || recording_failed "append the Python call site to $probe/call-sites.$run"
}
python_call_site_record "$@"
