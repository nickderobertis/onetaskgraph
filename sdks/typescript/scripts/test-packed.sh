#!/usr/bin/env bash
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
readonly BINARY="$ROOT/target/debug/onetaskgraph"
readonly TEMP="$(mktemp -d)"
trap 'rm -rf "$TEMP"' EXIT

cargo build --quiet -p onetaskgraph --bin onetaskgraph
bun run --cwd sdks/typescript build >/dev/null
(cd sdks/typescript && npm pack --silent --pack-destination "$TEMP" >/dev/null)
cd "$TEMP"
npm init --yes --silent >/dev/null
npm install --offline --omit=optional --ignore-scripts --silent ./onetaskgraph-sdk-*.tgz
mkdir project
printf '%s\n' '{"sources":{"work":{"plugin":"in-memory","config":{"tasks":[{"id":"T-1","title":"Packed","status":{"category":"todo","name":"Todo"},"labels":[]}]}}}}' > project/onetaskgraph.yaml
ONETASKGRAPH_BIN="$BINARY" node --input-type=module -e '
  import { OnetaskgraphClient } from "@onetaskgraph/sdk";
  const response = await new OnetaskgraphClient({ cwd: "project" }).taskList({ sources: ["work"] });
  if (response.items[0]?.item.title !== "Packed") process.exit(1);
'
