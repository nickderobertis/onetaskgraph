#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/releases/v0.1.0" "$tmp/canonical/v0.1.0" "$tmp/bin"
cat > "$tmp/fake" <<'FAKE'
#!/bin/sh
printf 'onetaskgraph 0.1.0\n'
FAKE
chmod +x "$tmp/fake"
target=x86_64-unknown-linux-gnu
case "$(uname -s):$(uname -m)" in Darwin:x86_64) target=x86_64-apple-darwin;; Darwin:arm64) target=aarch64-apple-darwin;; Linux:aarch64) target=aarch64-unknown-linux-gnu;; esac
name="onetaskgraph-v0.1.0-${target}.tar.gz"
tar -czf "$tmp/releases/v0.1.0/$name" -C "$tmp" fake --transform='s/fake/onetaskgraph/'
sha256sum "$tmp/releases/v0.1.0/$name" > "$tmp/canonical/v0.1.0/$name.sha256"
ONETASKGRAPH_VERSION=v0.1.0 ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh"
"$tmp/bin/onetaskgraph" | grep -qx 'onetaskgraph 0.1.0'
printf x >> "$tmp/releases/v0.1.0/$name"
if ONETASKGRAPH_VERSION=v0.1.0 ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then exit 1; fi
grep -q 'checksum mismatch' "$tmp/error"
if ONETASKGRAPH_VERSION=v0.1.0 ONETASKGRAPH_RELEASE_BASE_URL=https://mirror.example/releases ONETASKGRAPH_CHECKSUM_BASE_URL=https://mirror.example/checks "$root/scripts/install.sh" 2>"$tmp/error"; then exit 1; fi
grep -q "checksum shares the mirror's origin" "$tmp/error"
node_platform=$(node -p '`${process.platform}-${process.arch}`')
mkdir -p "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin"
cp "$tmp/fake" "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/onetaskgraph"
printf '{"name":"@onetaskgraph/cli-%s"}\n' "$node_platform" > "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/package.json"
(cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js") | grep -qx 'onetaskgraph 0.1.0'
