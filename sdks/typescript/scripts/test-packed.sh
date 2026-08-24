#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)" || {
  echo "test-packed: could not resolve the workspace; next: run from the checkout." >&2
  exit 1
}
readonly ROOT
readonly BINARY="$ROOT/target/debug/onetaskgraph"
CLI_VERSION="$(npm pkg get 'optionalDependencies.@onetaskgraph/cli' --prefix "$ROOT/sdks/typescript" | tr -d '"')" || {
  echo "test-packed: could not read the CLI dependency version; next: check package.json." >&2
  exit 1
}
if [[ ! "$CLI_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)?$ ]]; then
  echo "test-packed: CLI dependency version is not semantic; next: fix package.json." >&2
  exit 1
fi
readonly CLI_VERSION
TEMP="$(mktemp -d)" || {
  echo "test-packed: could not create a temporary directory; next: check TMPDIR." >&2
  exit 1
}
readonly TEMP
trap 'rm -rf "$TEMP"' EXIT

(cd sdks/typescript && npm pack --silent --pack-destination "$TEMP" >/dev/null) || {
  echo "test-packed: npm pack failed; next: run the sdk-typescript build target." >&2
  exit 1
}
cd "$TEMP" || {
  echo "test-packed: could not enter $TEMP; next: check temporary-directory access." >&2
  exit 1
}
npm init --yes --silent >/dev/null || {
  echo "test-packed: could not create the clean install project; next: check npm." >&2
  exit 1
}
npm install --offline --omit=optional --ignore-scripts --silent ./onetaskgraph-sdk-*.tgz || {
  echo "test-packed: local tarball install failed; next: inspect the package files." >&2
  exit 1
}
mkdir -p node_modules/@onetaskgraph/cli/bin || {
  echo "test-packed: could not construct the local CLI package; next: check temporary storage." >&2
  exit 1
}
printf '{"name":"@onetaskgraph/cli","version":"%s"}\n' "$CLI_VERSION" > node_modules/@onetaskgraph/cli/package.json || {
  echo "test-packed: could not write the local CLI manifest; next: check temporary storage." >&2
  exit 1
}
cp "$BINARY" node_modules/@onetaskgraph/cli/bin/onetaskgraph || {
  echo "test-packed: could not install the compiled CLI fixture; next: build the binary." >&2
  exit 1
}
chmod +x node_modules/@onetaskgraph/cli/bin/onetaskgraph || {
  echo "test-packed: could not make the CLI fixture executable; next: check file permissions." >&2
  exit 1
}
mkdir project || {
  echo "test-packed: could not create the fixture project; next: check temporary storage." >&2
  exit 1
}
printf '%s\n' '{"sources":{"work":{"plugin":"in-memory","config":{"tasks":[{"id":"T-1","title":"Packed","status":{"category":"todo","name":"Todo"},"labels":[]}]}}}}' > project/onetaskgraph.yaml || {
  echo "test-packed: could not write the fixture config; next: check temporary storage." >&2
  exit 1
}
if ! cat > packed-query.mjs <<'EOF'
  import { OnetaskgraphClient } from "@onetaskgraph/sdk";
  try {
    const response = await new OnetaskgraphClient({ cwd: "project" }).taskList({ sources: ["work"] });
    if (response.items[0]?.item.title !== "Packed") throw new Error("packed query returned the wrong task");
  } catch (error) {
    console.error(`test-packed: ${error instanceof Error ? error.message : String(error)}`);
    process.exit(1);
  }
EOF
then
  echo "test-packed: could not write the query program; next: check temporary storage." >&2
  exit 1
fi
env -u ONETASKGRAPH_BIN node packed-query.mjs || {
  echo "test-packed: installed SDK query failed; next: run sdk-typescript:test." >&2
  exit 1
}
