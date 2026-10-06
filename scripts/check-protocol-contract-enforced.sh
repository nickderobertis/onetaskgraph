#!/usr/bin/env bash
# Watch scripts/check-protocol-contract.sh refuse a drifted list of the methods the protocol
# does not carry, and a wire member of a struct whose fields are private that the document
# stops naming.
#
# That guard exempts the template operations from §4's method table, and holds the exemption
# to the list docs/plugin-protocol.md gives of them, both ways. An exemption is exactly the
# part of a guard nobody notices going quiet: were the reconciliation to stop reading the
# list, a method dropped from it — or a name added to it that no exemption backs — would pass
# in silence. Each case below puts one such drift to the WORKING tree's guard, over a scratch
# copy of the files it reads, and asserts on the refusal and on its words.
set -euo pipefail

fatal() {
  echo "check-protocol-contract-enforced: $1" >&2
  echo "check-protocol-contract-enforced: next: $2" >&2
  exit 1
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just check' does"
readonly ROOT

readonly GUARD="scripts/check-protocol-contract.sh"
readonly DOCUMENT="docs/plugin-protocol.md"
readonly SUBPROCESS="crates/onetaskgraph-core/src/subprocess"

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree these cases run in" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

# Exactly what the guard reads, laid out where it looks for it.
mkdir -p "$scratch/scripts" "$scratch/docs" "$scratch/$SUBPROCESS" \
  "$scratch/crates/onetaskgraph-plugin-api" || fatal \
  "could not lay out the scratch tree at $scratch" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
copy_failed() {
  fatal "could not copy $1 into $scratch" \
    "check that $1 is present in this checkout, then rerun"
}
cp "$ROOT/$GUARD" "$scratch/$GUARD" || copy_failed "$GUARD"
cp "$ROOT/$DOCUMENT" "$scratch/$DOCUMENT.held" || copy_failed "$DOCUMENT"
cp -R "$ROOT/crates/onetaskgraph-plugin-api/src" "$scratch/crates/onetaskgraph-plugin-api/src" ||
  copy_failed "crates/onetaskgraph-plugin-api/src"
for file in connection.rs source.rs plugin.rs wire.rs; do
  cp "$ROOT/$SUBPROCESS/$file" "$scratch/$SUBPROCESS/$file" || copy_failed "$SUBPROCESS/$file"
done

# Run the guard over the document as `edit` (a sed program) leaves it, and answer with what
# it wrote to standard error, failing unless it exited as `expected` says.
run_case() {
  local name="$1" edit="$2" expected="$3"
  local remedy="${4:-}"
  if [ -z "$remedy" ]; then
    remedy="restore the reconciliation of NOT_CARRIED_REASON against §4's list in $GUARD"
  fi
  sed "$edit" "$scratch/$DOCUMENT.held" >"$scratch/$DOCUMENT" || fatal \
    "case '$name': could not write the edited document" \
    "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
  if cmp -s "$scratch/$DOCUMENT" "$scratch/$DOCUMENT.held" && [ -n "$edit" ]; then
    fatal "case '$name': the edit changed nothing, so this case would prove nothing" \
      "update the edit in $0 to match the list in $DOCUMENT as it now reads"
  fi
  local status=0
  said="$(bash "$scratch/$GUARD" 2>&1 >/dev/null)" || status=$?
  if [ "$expected" = pass ] && [ "$status" -ne 0 ]; then
    fatal "case '$name': the guard refused the unedited document: $said" \
      "run 'bash $GUARD' in the working tree and fix what it names first"
  fi
  if [ "$expected" = refuse ] && [ "$status" -eq 0 ]; then
    fatal "case '$name': the guard passed a drifted document" "$remedy"
  fi
}

# Holds a case's refusal to naming what drifted, so a refusal for some other reason cannot
# stand in for the one this case is about.
names() {
  local name="$1" needle="$2"
  local remedy="${3:-make $GUARD name the method and the side it is missing from}"
  case "$said" in
    *"$needle"*) ;;
    *) fatal "case '$name': the refusal did not say \"$needle\": $said" "$remedy" ;;
  esac
}

said=""
run_case "unedited" "" pass

run_case "a method missing from the list" \
  's/^- `set_task_rendering`, `set_project_rendering` and `set_document_rendering`, a regenerate in$/- `set_task_rendering` and `set_project_rendering`, a regenerate in/' \
  refuse
names "a method missing from the list" \
  'template operation not carried `set_document_rendering` is declared in `NOT_METHODS`'

run_case "a name no exemption backs" \
  's/^- `write_task_rendered`, `write_project_rendered` and `write_document_rendered`, a create from$/- `write_task_rendered`, `write_project_rendered`, `write_document_rendered` and `write_label_rendered`, a create from/' \
  refuse
names "a name no exemption backs" \
  'template operation not carried `write_label_rendered` is specified by docs/plugin-protocol.md'

run_case "the list gone" \
  's/^The template operations are not carried\./The template operations are elsewhere./' \
  refuse
names "the list gone" 'no longer says "The template operations are not carried."'

# `MetadataMatch` keeps every field private behind a validating constructor, so the guard
# reads its members from the private fields; were it to stop, a member dropped from §4.5
# would pass in silence.
run_case "a private wire member the document stops naming" \
  's/"path": \["root_cause"\], //; s/— `path`, zero or more nested object keys/— a path, zero or more nested object keys/' \
  refuse \
  "restore the reading of private as well as public members in STRUCT_SECTIONS' loop in $GUARD"
names "a private wire member the document stops naming" \
  '`MetadataMatch` carries the field "path"' \
  "make $GUARD name the struct and the member the section no longer specifies"
