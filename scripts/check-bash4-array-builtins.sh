#!/usr/bin/env bash
# Refuse `mapfile` and `readarray` anywhere under scripts/.
#
# They are one bash 4 builtin under two names, and macos-latest runs bash 3.2. Every script
# here declares `#!/usr/bin/env bash`, so on that runner each one IS a 3.2 script: the line
# reaching the builtin ends it with "mapfile: command not found", and the job reports
# whatever the script was proving having gone wrong. It stays hidden until a macOS job
# enters the script carrying it — which is how four such reads sat on main until an
# unrelated fix let `install path (macos-latest)` run far enough to reach one.
#
# Write `read_lines <array-name>` from scripts/read-lines.sh instead: same array, on 3.2.
set -euo pipefail

fatal() {
  echo "check-bash4-array-builtins: $1" >&2
  echo "check-bash4-array-builtins: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just script-check' does"
readonly ROOT
cd "$ROOT" || fatal \
  "could not enter $ROOT to scan its scripts" \
  "check that directory's permissions, then rerun"

# The scan refuses with a status of its own rather than sharing python's. An unhandled
# exception exits 1 too, so a refusal spelled 1 would be indistinguishable from the scan
# having died partway through — and this check would then exit on a bare traceback, having
# cleared every script it never reached, with none of the next action it gives otherwise.
scan_status=0
python3 - <<'PY' || scan_status=$?
import re
import sys
from pathlib import Path

# Command position, judged by what precedes the name: a full-line comment is skipped, and a
# name that opens a quoted string is preceded by that quote and so reads as data. Past that
# this is a line scan rather than a shell parser, so a name reached after a space INSIDE a
# quoted string is refused along with a real call — conservative in the safe direction. A
# script that has to write one of these names as data should build it from a variable, as
# scripts/check-bash4-array-builtins-enforced.sh does.
#
# The names are joined into the pattern rather than written inside it because this scan
# reads every shell script here, itself included: a literal alternation would sit in
# command position on its own line and match itself.
BUILTINS = ("mapfile", "readarray")
# The status this scan exits with when it has something to report, kept away from 1 so an
# unhandled exception cannot be read as a refusal that already explained itself.
REFUSED = 2
LOOKS_LIKE_A_CALL = re.compile(
    r"(?:^|[\s;&|(`])(" + "|".join(BUILTINS) + r")(?![\w])"
)

unreadable = []


# str(Path) joins with os.sep, so this same scan names a file "scripts/x.sh" on Linux and
# macOS and "scripts\x.sh" on Windows, and every check here is required on all three. The
# separator is not cosmetic: `just` runs bash on Windows too, so a backslash path is not
# one the reader can paste back into the shell they are standing in — and a check that
# asserts on this report would have to spell every path twice to stay platform-blind.
def display_path(path):
    return path.as_posix()


# Every shell script here, found by extension or by shebang, so a new one that arrives
# without a .sh suffix is still covered.
def shell_scripts():
    for path in sorted(Path("scripts").rglob("*")):
        if not path.is_file():
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError) as error:
            # Not skipped quietly: a file this scan cannot read is a file it cannot clear,
            # and a guard that passes over what it never checked is the failure it exists
            # to prevent.
            unreadable.append((path, error))
            continue
        first = text.split("\n", 1)[0]
        if path.suffix == ".sh" or (first.startswith("#!") and "bash" in first):
            yield path, text


problems = []
for path, text in shell_scripts():
    for number, line in enumerate(text.split("\n"), start=1):
        if line.lstrip().startswith("#"):
            continue
        found = LOOKS_LIKE_A_CALL.search(line)
        if found:
            problems.append((path, number, found.group(1), line.strip()))

for path, error in unreadable:
    shown = display_path(path)
    print(f"check-bash4-array-builtins: {shown}: could not be read as text, so it was not",
          file=sys.stderr)
    print(f"check-bash4-array-builtins: scanned: {error}", file=sys.stderr)
    print("check-bash4-array-builtins: next: restore it as readable UTF-8, or move it out",
          file=sys.stderr)
    print("check-bash4-array-builtins: of scripts/ if it is not a script.", file=sys.stderr)

if problems:
    for path, number, builtin, line in problems:
        location = f"{display_path(path)}:{number}"
        print(f"check-bash4-array-builtins: {location}: {builtin} is a bash 4 builtin:",
              file=sys.stderr)
        print(f"check-bash4-array-builtins:     {line}", file=sys.stderr)
    print("check-bash4-array-builtins: macos-latest ships bash 3.2, where that line ends the",
          file=sys.stderr)
    print("check-bash4-array-builtins: script with 'command not found' — and 'check",
          file=sys.stderr)
    print("check-bash4-array-builtins: (macos-latest)' and 'install path (macos-latest)' are",
          file=sys.stderr)
    print("check-bash4-array-builtins: required checks.", file=sys.stderr)
    print("check-bash4-array-builtins: next: source scripts/read-lines.sh and read the lines",
          file=sys.stderr)
    print("check-bash4-array-builtins: with", file=sys.stderr)
    print("check-bash4-array-builtins:   read_lines <array-name> < <source>", file=sys.stderr)
    print("check-bash4-array-builtins: which builds the same array, and runs on 3.2.",
          file=sys.stderr)

# 2, not 1: see the case below. 1 is what python exits with when this scan raises, and the
# two must not arrive at the same place.
if problems or unreadable:
    raise SystemExit(REFUSED)
PY

case "$scan_status" in
  0) ;;
  2) exit 1 ;;
  *) fatal \
    "python3 ended with status $scan_status, so the scripts were not all scanned; any\
 traceback above says where it stopped" \
    "fix what the traceback names, or run 'python3 --version' to confirm a working python3\
 is on PATH, then rerun" ;;
esac
