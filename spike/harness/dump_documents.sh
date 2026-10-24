#!/usr/bin/env bash
# Regenerate documents.json: every GraphQL document the GitHub Projects plugin sends, by the
# name of its constant, read out of the compiled crate at a given onetaskgraph checkout (the
# committed table is from the v0.7.1 tag). The documents are anonymous, so this table is how
# the proxy names each request it records. Usage: dump_documents.sh <checkout>
set -euo pipefail
checkout="$1"
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
crate="$checkout/crates/onetaskgraph-github-projects"
names=$(grep -oE '^    pub const [A-Z_]+: &str' "$crate/src/lib.rs" | awk '{print $3}' | tr -d ':' \
    | grep -vE '^(KIND|DESIGN_TITLE_PREFIX|PRIORITY_FIELD)$')
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
(cd "$checkout" && cargo run -q --release -p onetaskgraph-github-projects --example dump_documents) \
    > "$here/documents.json"
