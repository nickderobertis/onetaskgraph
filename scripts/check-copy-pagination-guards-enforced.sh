#!/usr/bin/env bash
# Watch scripts/check-copy-pagination-guards.sh refuse what it is meant to refuse.
#
# That guard reads a rule off each pagination loop's own text, and a text scan keeps
# passing after it has stopped matching what it describes. Its rule has two halves and the
# second one is the one nobody would notice going missing: a loop reading its page from a
# verb of this engine takes its bound from that verb, so if someone later points such a
# loop straight at a plugin the page bound becomes that loop's own again — and the guard
# has to start demanding it. Case by case below, in a scratch tree, both halves are put to
# it, along with the loop it must NOT demand a bound of and the second spelling it refuses.
#
# The fixtures are written here rather than mutated out of the real engine source: what is
# under test is the rule the guard applies, and the real file is what the guard already
# reads on every gate. Keeping the cases synthetic is also what keeps this project reading
# nothing outside `scripts/` — an input from a crate is an edge this project refuses.
set -euo pipefail

fatal() {
  echo "check-copy-pagination-guards-enforced: $1" >&2
  echo "check-copy-pagination-guards-enforced: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just script-check' does"
readonly ROOT

readonly GUARD="check-copy-pagination-guards.sh"

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree these cases are planted in" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

# The WORKING tree's guard, not HEAD's: an author repairing it must not watch this keep
# failing against the version they just replaced.
mkdir -p "$scratch/scripts" "$scratch/crates/onetaskgraph-core/src/engine" || fatal \
  "could not lay out the scratch tree at $scratch" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
cp "$ROOT/scripts/$GUARD" "$scratch/scripts/$GUARD" || fatal \
  "could not copy scripts/$GUARD into $scratch" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"

readonly ENGINE="$scratch/crates/onetaskgraph-core/src/engine"

# Both refusals, where the guard insists they are implemented once.
plant_authority() {
  cat >"$ENGINE/fetch.rs" <<'RS' || fatal \
    "could not write the fixture standing in for engine/fetch.rs" \
    "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
pub(crate) fn fits(returned: usize, asked_for: u32) -> Result<(), SourceError> {
    unimplemented!()
}

pub(crate) fn advances<T: PartialEq>(returned: Option<&T>, asked: Option<&T>, scan: &str) {
    unimplemented!()
}
RS
}

# Two loops, one of each shape the rule rules on: a walk that pages by a verb of this
# engine, and a walk that reads a plugin's page directly.
plant_copy() {
  cat >"$ENGINE/copy.rs" <<'RS' || fatal \
    "could not write the fixture standing in for engine/copy.rs" \
    "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
impl Engine {
    async fn members(&self, project: &GlobalId) -> Result<Vec<GlobalId>, EngineError> {
        loop {
            let asked = request.paging.token.clone();
            let response = self.tasks(&request).await?;
            advances(
                response.next.as_ref(),
                asked.as_ref(),
                "the tasks of a project were being read for a copy",
            )?;
            match response.next {
                Some(token) => request.paging.token = Some(token),
                None => return Ok(members),
            }
        }
    }

    async fn orphans(&self, destination: &ResolvedSource) -> Result<Vec<CopyOutcome>, EngineError> {
        loop {
            let asked = cursor.clone();
            let request = request_for(destination, cursor);
            let page = destination.source().query_tasks(&query, &request).await?;
            fits(page.items.len(), request.limit)?;
            advances(
                page.next.as_ref(),
                asked.as_ref(),
                "the destination was being read for items the copy left behind",
            )?;
            match page.next {
                Some(next) => cursor = Some(next),
                None => return Ok(orphans),
            }
        }
    }
}
RS
}

# Replace one literal substring of a scratch fixture. python3 rather than `sed -i`, whose
# in-place spelling differs between GNU and BSD and so would fail on the macOS runner.
substitute() {
  python3 - "$ENGINE/$1" "$2" "$3" <<'PY' || fatal \
    "the helper that rewrites a scratch fixture did not finish, so that case was never put to the guard" \
    "run 'python3 --version' to confirm a working python3 is on PATH, then rerun"
import pathlib
import sys

path, before, after = pathlib.Path(sys.argv[1]), sys.argv[2], sys.argv[3]
text = path.read_text(encoding="utf-8")
if before not in text:
    print(f"the fixture {path} does not contain the text this case rewrites", file=sys.stderr)
    raise SystemExit(1)
path.write_text(text.replace(before, after, 1), encoding="utf-8")
PY
}

failures=0
GUARD_OUTPUT=""
GUARD_STATUS=0

run_guard() {
  GUARD_OUTPUT="$(bash "$scratch/scripts/$GUARD" 2>&1)" && GUARD_STATUS=0 || GUARD_STATUS=$?
}

# A case the guard must refuse, saying `$2` while it does.
refuses() {
  local what="$1" wanted="$2"
  run_guard
  if [ "$GUARD_STATUS" -eq 0 ]; then
    echo "check-copy-pagination-guards-enforced: $GUARD passed $what" >&2
    failures=$((failures + 1))
    return
  fi
  case "$GUARD_OUTPUT" in
    *"$wanted"*) ;;
    *)
      echo "check-copy-pagination-guards-enforced: $GUARD refused $what without saying '$wanted'" >&2
      printf '%s\n' "$GUARD_OUTPUT" >&2
      failures=$((failures + 1))
      ;;
  esac
}

# A case the guard must accept, because a guard that refuses everything proves nothing.
accepts() {
  local what="$1"
  run_guard
  if [ "$GUARD_STATUS" -ne 0 ]; then
    echo "check-copy-pagination-guards-enforced: $GUARD refused $what, which it must accept" >&2
    printf '%s\n' "$GUARD_OUTPUT" >&2
    failures=$((failures + 1))
  fi
}

# The baseline. Both loop shapes, each holding exactly the half of the rule it owes.
plant_authority
plant_copy
accepts "a copy path where each loop holds the half of the rule it owes"

# The half that matters: a loop that stops reading a verb of this engine and starts reading
# a plugin owes the page bound from that moment, and nothing else in the file changed.
plant_copy
substitute copy.rs "self.tasks(&request)" "self.resolved(project)?.source().query_tasks(&query, &request)"
refuses "a walk repointed from an engine verb at a plugin, with no page bound" \
  "reads a plugin's page without the page bound"

# The other half: a bound restated on a page this engine's own verb already bounded reads
# like the guard and can never fail, so it is refused rather than tolerated.
plant_copy
substitute copy.rs "            advances(
                response.next.as_ref()," "            fits(response.items.len(), PROJECT_PAGE.get())?;
            advances(
                response.next.as_ref(),"
refuses "a page bound restated on a page an engine verb already bounded" \
  "bounds a page this engine's own verb already bounded"

# A plugin-reading loop with its page bound taken away.
plant_copy
substitute copy.rs "            fits(page.items.len(), request.limit)?;
" ""
refuses "a plugin-reading walk with no page bound" "reads a plugin's page without the page bound"

# Either loop shape with its cursor-repeat guard taken away: that half is owed by both.
plant_copy
substitute copy.rs "            advances(
                response.next.as_ref(),
                asked.as_ref(),
                \"the tasks of a project were being read for a copy\",
            )?;
" ""
refuses "a walk over an engine verb with no cursor-repeat guard" \
  "paginates without the cursor-repeat guard"

plant_copy
substitute copy.rs "            advances(
                page.next.as_ref(),
                asked.as_ref(),
                \"the destination was being read for items the copy left behind\",
            )?;
" ""
refuses "a plugin-reading walk with no cursor-repeat guard" "paginates without the cursor-repeat guard"

# Either refusal spelled a second time on the copy path rather than called.
plant_copy
substitute copy.rs "impl Engine {" "// the source returned the cursor it was given while a copy was walking it
impl Engine {"
refuses "the cursor-repeat refusal spelled a second time on the copy path" "spells"

# Neither refusal implemented where the copy path shares it from.
plant_copy
plant_authority
python3 - "$ENGINE/fetch.rs" <<'PY' || fatal \
  "could not take the shared page bound out of the fetch.rs fixture" \
  "run 'python3 --version' to confirm a working python3 is on PATH, then rerun"
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
path.write_text(path.read_text(encoding="utf-8").replace("pub(crate) fn fits", "fn fits"), encoding="utf-8")
PY
refuses "a page bound with no single implementation to share" "has no single implementation"

# A copy path with no loop in it at all, which is a check that read nothing rather than a
# copy path that is safe.
plant_authority
printf 'impl Engine {}\n' >"$ENGINE/copy.rs"
refuses "a copy path holding no loop at all" "contains no loop at all"

if [ "$failures" -ne 0 ]; then
  fatal "$failures case(s) of $GUARD did not behave as this check requires" \
    "read the case names above: each names the copy-path shape the guard was given. Repair scripts/$GUARD so it refuses that shape — a guard nobody has watched fail is a guard nobody knows works."
fi
