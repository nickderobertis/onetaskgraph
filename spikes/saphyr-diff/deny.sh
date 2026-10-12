#!/usr/bin/env bash
# SPIKE (serde-yaml-replacement, not for landing): what serde-saphyr 1.3 and saphyr 0.2 bring
# into this workspace's dependency graph, judged by its own deny.toml.
#
#   spikes/saphyr-diff/deny.sh <out-dir>
#
# Prints the crates the two add to Cargo.lock against main, whether any of the libyaml family
# is reachable from either, and cargo-deny's bans, licenses, sources and advisories verdicts
# over the workspace with both in it (the harness crate is a workspace member and depends on
# both; core and local-md reach serde-saphyr through `yaml-spike`, which deny.toml's
# `all-features = true` turns on).
set -uo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
out="${1:?usage: deny.sh <out-dir>}"
mkdir -p "$out"
cd "$repo"

echo "deny: crates added to Cargo.lock against main:"
git diff "$(git merge-base HEAD main)" -- Cargo.lock | grep -A1 '^+name = ' | grep -E '^\+(name|version)' \
  | paste - - | sed -E 's/^\+name = "([^"]*)"\s+\+version = "([^"]*)"/  \1 \2/'

echo "deny: the libyaml family reachable from either candidate (none expected):"
family='^(serde_yaml|serde_yaml_ng|serde_norway|serde_yml|unsafe-libyaml|unsafe-libyaml-norway|libyml) '
for crate in serde-saphyr saphyr; do
  cargo tree -q -p "$crate" -e normal,build --prefix none 2>/dev/null | sort -u >"$out/deny-tree-$crate.txt"
  hits="$(grep -E "$family" "$out/deny-tree-$crate.txt" || true)"
  printf '  %s (%s crates in its closure): %s\n' "$crate" "$(wc -l <"$out/deny-tree-$crate.txt")" "${hits:-none}"
done

echo "deny: cargo deny check over the workspace:"
cargo deny --all-features check bans licenses sources advisories >"$out/deny.log" 2>&1
echo "  exit $? — full output in $out/deny.log; its error and warning headlines:"
grep -E '^(error|warning)\[' "$out/deny.log" | sort | uniq -c | sed 's/^/  /'
echo "  findings naming a crate either candidate brings:"
grep -nE 'serde-saphyr|saphyr|granit-parser|annotate-snippets|encoding_rs_io|zmij|nohash-hasher|ordered-float|hashlink|arraydeque|anstyle|unicode-width' "$out/deny.log" \
  | grep -E 'error|warning|├|└|─' | head -40 | sed 's/^/  /'

# The banned edge above, if any, is the harness's own (it depends on the engine, which
# deny.toml permits only named wrappers). Without it in the graph, serde-saphyr is still
# there through core's and local-md's `yaml-spike`; saphyr is not.
echo "deny: cargo deny check with the harness crate excluded:"
cargo deny --all-features --exclude saphyr-diff check bans licenses sources advisories >"$out/deny-excluded.log" 2>&1
echo "  exit $? — full output in $out/deny-excluded.log; its error headlines:"
grep -E '^error\[' "$out/deny-excluded.log" | sort | uniq -c | sed 's/^/  /'
grep -q 'serde-saphyr' "$out/deny-excluded.log" && echo "  (the log names serde-saphyr)" || true
