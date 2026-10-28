#!/usr/bin/env bash
# llmlint: ignore-file[new_code_lands_in_a_project] This is the measurement record beside the captures it produced: run by hand against GitHub through measure.sh, never by a target. docs/measurements is no Nx project (nor is coalesced-members-dry-run/dry_run.py beside it), and making it one would put a live-GitHub harness inside affected selection.
# llmlint: ignore-file[boundary_inputs_validated] Every input here is this harness's own raw log, its own state folder or the plan it generated moments earlier; the committed captures are validated where they are read, by analyze.py, and what leaves the raw logs is held by the sanitizer's allowlist and leak scan.
# llmlint: ignore-file[changed_behavior_has_e2e] Its proof is the live run itself, whose captures are committed and reconciled by test_analyze.py; the GitHub-facing parts can be driven only against GitHub's test board, which measure.sh does, and the sanitizer produces every capture those tests read.
# llmlint: ignore-file[contracts_have_one_source_or_a_drift_gate] What this mirrors (the plugin's documents, the host's configuration, the scratch identities) is pinned to the releases measured, onetaskgraph v0.12.7 and ai-orchestrator 668dc496, each stated in the report and in every row's versions.json; the capture schema it shares with analyze.py is held by test_analyze.py, which builds every capture through sanitize.write.
# Regenerate documents.json: every GraphQL document the GitHub Projects plugin sends, by the
# name of its constant, read out of the compiled crate at a given onetaskgraph checkout (the
# committed table is from the v0.12.7 tag). The documents are anonymous, so this table is how
# the proxy names each request it records. Usage: dump_documents.sh <checkout>
set -euo pipefail
if [ "$#" -ne 1 ] || [ ! -d "$1/crates/onetaskgraph-github-projects" ]; then
    echo "usage: $0 <onetaskgraph checkout>; name a checkout of the tag the measurement runs (git archive v0.12.7 | tar -x -C <dir>)" >&2
    exit 2
fi
checkout="$1"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
crate="$checkout/crates/onetaskgraph-github-projects"
names=$(grep -oE '^    pub const [A-Z_]+: &str' "$crate/src/lib.rs" | awk '{print $3}' | tr -d ':' \
    | grep -vE '^(KIND|DESIGN_TITLE_PREFIX|PRIORITY_FIELD)$' || true)
if [ -z "$names" ]; then
    echo "dump_documents.sh: no \`pub const NAME: &str\` document found in $crate/src/lib.rs; name a checkout of the measured tag, or update the pattern if the plugin moved its documents" >&2
    exit 1
fi
mkdir -p "$crate/examples"
trap 'rm -rf "$crate/examples"' EXIT
{
    echo 'use onetaskgraph_github_projects::graphql::*;'
    echo 'fn main() { let mut m = serde_json::Map::new();'
    for name in $names; do
        echo "m.insert(\"$name\".into(), serde_json::Value::String($name.to_owned()));"
    done
    echo 'println!("{}", serde_json::to_string_pretty(&serde_json::Value::Object(m)).unwrap()); }'
} > "$crate/examples/dump_documents.rs"
(cd "$checkout" && cargo run -q -p onetaskgraph-github-projects --example dump_documents) \
    > "$here/documents.json"
