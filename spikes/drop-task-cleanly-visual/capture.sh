#!/usr/bin/env bash
# SPIKE MOCK (drop-task-cleanly spike-visual). Throwaway: never lands.
#
# Re-captures every image of the report `drop-task-cleanly-spike-visual-report`:
#   bash spikes/drop-task-cleanly-visual/capture.sh [BASE_BINARY]
#
# BASE_BINARY is an `onetaskgraph` built from the unchanged base (de029a41); the "before"
# of the copy report is taken with it. Without it, the script builds the base itself in a
# scratch worktree. The mock binary is this tree's debug build.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)"
cd "$ROOT"
OUT="$ROOT/spikes/drop-task-cleanly-visual/shots"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

for v in $(compgen -e); do case "$v" in ONETASKGRAPH_*) [ "$v" = ONETASKGRAPH_TOOLS_HOME ] || unset "$v" ;; esac; done

cargo build --locked --bin onetaskgraph >&2
MOCK="$WORK/onetaskgraph-mock"
cp target/debug/onetaskgraph "$MOCK"

BASE="${1:-}"
if [ -z "$BASE" ]; then
  git worktree add --detach "$WORK/base" de029a41 >&2
  (cd "$WORK/base" && CARGO_TARGET_DIR="$ROOT/target/spike-base" cargo build --locked --bin onetaskgraph >&2)
  BASE="$ROOT/target/spike-base/debug/onetaskgraph"
  git worktree remove --force "$WORK/base"
fi

FREEZE="$(bash scripts/screenshots-freeze.sh resolve)"
FONT="$ROOT/screenshots/fonts/JetBrainsMono-Regular.ttf"
flags=(--language ansi --font.file "$FONT" --font.family "JetBrains Mono" --font.size 14
  --window --background "#0d1117" --padding "20,30" --margin 0 --border.radius 8
  --width 843 --wrap 93)

export HOME="$WORK/home" XDG_CONFIG_HOME="$WORK/home/.config" XDG_CACHE_HOME="$WORK/home/.cache" \
  XDG_DATA_HOME="$WORK/home/.local/share" XDG_STATE_HOME="$WORK/home/.local/state" TMPDIR="$WORK/tmp"
mkdir -p "$HOME/.config" "$TMPDIR" "$OUT"

# The fixture state: screenshots/fixture/sync, plus one done task in the plan, copied once
# into `work` so every task of plans:P-1 has a counterpart there.
STATE="$WORK/state"
cp -R screenshots/fixture/sync "$STATE"
cat >"$STATE/plans/tasks/T-4.md" <<'MD'
---
title: Pin the client version
status: done
labels: [chore]
project: P-1
---
Shipped before the plan changed.
MD
(cd "$STATE" && "$BASE" project copy plans:P-1 --to work >/dev/null)

# shot NAME DIR BINARY ARGS... — run one command, render "$ command" and what it wrote.
shot() {
  local name="$1" dir="$2" bin="$3"; shift 3
  local text="$WORK/$name.txt" status=0
  printf '$ onetaskgraph %s\n' "$*" >"$text"
  (cd "$dir" && "$bin" "$@") >>"$text" 2>&1 || status=$?
  [ "$status" -eq 0 ] || printf '\n(exit %s)\n' "$status" >>"$text"
  for ext in svg png; do
    "$FREEZE" "$text" "${flags[@]}" -o "$OUT/$name.$ext" </dev/null >&2
  done
}

# 2. The refusal: plans:T-1 depends on plans:T-2.
shot task-delete-refused "$STATE" "$MOCK" task delete plans:T-2
# 1. A delete of a task nothing depends on.
shot task-delete "$STATE" "$MOCK" task delete plans:T-1
# The done task is dropped too (not shown), so the re-copy meets one open and one done orphan.
(cd "$STATE" && "$MOCK" task delete plans:T-4 >/dev/null)

# 3. The re-copy, before (base, today) and after (mock), each over its own copy of one state.
cp -R "$STATE" "$WORK/before"
cp -R "$STATE" "$WORK/after"
shot project-copy-before "$WORK/before" "$BASE" project copy plans:P-1 --to work
shot project-copy-after "$WORK/after" "$MOCK" project copy plans:P-1 --to work --close-orphans
echo "wrote $OUT" >&2
