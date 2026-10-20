#!/usr/bin/env bash
# Watch scripts/check-protocol-contract.sh refuse a drifted list of the methods the protocol
# does not carry, a wire member of a struct whose fields are private that the document
# stops naming, and a payload example or comment member table drifted from its struct.
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

# The payloads §4.4a and §4.16a show as JSON are held to the structs that read them both ways,
# and a comment's members to §4.15's table; were either reading to stop, a member an example
# still sends after the struct dropped it, or one the struct gained, would pass in silence.
run_case "an example member no struct reads" \
  's/"params": { "id": "ENG-1", "comments": {/"params": { "id": "ENG-1", "stale": 1, "comments": {/' \
  refuse \
  "restore the reconciliation of PAYLOAD_EXAMPLES against the example's keys in $GUARD"
names "an example member no struct reads" \
  'shows `params.stale`, but `TaskDetailParams` has no such field'

run_case "a payload member the section stops naming" \
  's/"author": "someone", "created_at"/"created_at"/' \
  refuse \
  "restore the reconciliation of PAYLOAD_EXAMPLES against the struct's fields in $GUARD"
names "a payload member the section stops naming" \
  '`CommentOrigin` carries the field "author" but "### 4.16a `write_comment_copy`"'

# A contradiction's `item` is `#[serde(flatten)]`, so what crosses is its variant's member —
# `task` or `project` — and the guard reads those from `ContradictedItem`; were it to read the
# Rust field name instead, a section that stopped naming the `project` member would pass.
run_case "a flattened member the section stops naming" \
  '/^### 4.23 /,/^### 4.24 /{s/"project"/"held"/g;s/`project`/`held`/g;}' \
  refuse \
  "restore the reading of a flattened field as its enum's variants in struct_fields in $GUARD"
names "a flattened member the section stops naming" \
  '`StatusContradiction` carries the field "project" but "### 4.23 `status_contradictions`"'

run_case "a comment member gone from its table" \
  's/^| `origin` | object, optional | Present only on a comment a copy wrote/| `source` | object, optional | Present only on a comment a copy wrote/' \
  refuse \
  "restore the reconciliation of \`Comment\` against §4.15's member table in $GUARD"
names "a comment member gone from its table" \
  'comment member `origin` is declared in `Comment`'
names "a comment member gone from its table" \
  'comment member `source` is specified by docs/plugin-protocol.md'

# `ClosedAs` is read from issue.rs beside the other kebab-case enums, and `Completion`'s
# variants and members from §4.24's bullets; were either reading to stop, a closure value or a
# holding's member the section stops spelling would pass in silence.
run_case "a closure value the section stops spelling" \
  's/`"done"` or `"cancelled"`, the only two/`"done"` or cancelled, the only two/' \
  refuse \
  "restore issue.rs in the enum inventory and ClosedAs in ENUM_SECTIONS in $GUARD"
names "a closure value the section stops spelling" \
  '`ClosedAs` serializes the value "cancelled"'

run_case "a holding member gone from its bullet" \
  's/^- `unchanged`, with `task` — /- `unchanged` — /' \
  refuse \
  "restore the reconciliation of \`Completion\` against §4.24's bullets in $GUARD"
names "a holding member gone from its bullet" \
  '`Completion::unchanged` carries the member `task`'

run_case "a holding variant the section does not declare" \
  's/^- `absent` — this source/- `withdrawn` — this source/' \
  refuse \
  "restore the reconciliation of \`Completion\` against §4.24's bullets in $GUARD"
names "a holding variant the section does not declare" \
  'completion `withdrawn`'
