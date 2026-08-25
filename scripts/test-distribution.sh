#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$root/crates/onetaskgraph/Cargo.toml" | head -n1)
tag="v$version"
mkdir -p "$tmp/releases/$tag" "$tmp/canonical/$tag" "$tmp/bin"
cat > "$tmp/fake" <<FAKE
#!/bin/sh
if [ "\${1:-}" = --exit ]; then exit "\$2"; fi
printf 'onetaskgraph $version\n'
FAKE
chmod +x "$tmp/fake"
target=x86_64-unknown-linux-gnu
case "$(uname -s):$(uname -m)" in Darwin:x86_64) target=x86_64-apple-darwin;; Darwin:arm64) target=aarch64-apple-darwin;; Linux:aarch64) target=aarch64-unknown-linux-gnu;; esac
name="onetaskgraph-${tag}-${target}.tar.gz"
tar -czf "$tmp/releases/$tag/$name" -C "$tmp" fake --transform='s/fake/onetaskgraph/'
sha256sum "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"
ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh"
"$tmp/bin/onetaskgraph" | grep -qx "onetaskgraph $version" || { echo "installed command did not report $version; next: inspect the locally archived binary" >&2; exit 1; }
ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" "$root/scripts/install.sh" --version "$tag" --to "$tmp/bin" --archive-base-url "file://$tmp/releases" >/dev/null
"$root/scripts/install.sh" --help | grep -q 'exit codes:' || { echo "installer help omitted exit codes; next: update its public usage contract" >&2; exit 1; }
if "$root/scripts/install.sh" --unknown 2>"$tmp/error"; then echo "unknown option was accepted; next: inspect argument parsing" >&2; exit 1; fi
printf x >> "$tmp/releases/$tag/$name"
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "tampered archive installed; next: inspect checksum verification" >&2; exit 1; fi
grep -q 'checksum mismatch' "$tmp/error" || { echo "tamper failure omitted checksum mismatch; next: inspect installer diagnostics" >&2; exit 1; }
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL=https://mirror.example/releases ONETASKGRAPH_CHECKSUM_BASE_URL=https://mirror.example/checks "$root/scripts/install.sh" 2>"$tmp/error"; then echo "mirror-controlled checksum was accepted; next: inspect origin comparison" >&2; exit 1; fi
grep -q "checksum shares the mirror's origin" "$tmp/error" || { echo "mirror rejection omitted its reason; next: inspect installer diagnostics" >&2; exit 1; }
if "$root/scripts/install.sh" --version 2>"$tmp/error"; then echo "missing option value was accepted; next: inspect installer argument parsing" >&2; exit 1; fi
grep -q 'requires a value' "$tmp/error" || { echo "missing-value failure omitted its reason; next: inspect argument diagnostics" >&2; exit 1; }
if ONETASKGRAPH_VERSION="${tag}junk" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "malformed tag was accepted; next: inspect tag validation" >&2; exit 1; fi
node_platform=$(node -p '`${process.platform}-${process.arch}`')
mkdir -p "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin"
cp "$tmp/fake" "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/onetaskgraph"
printf '{"name":"@onetaskgraph/cli-%s"}\n' "$node_platform" > "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/package.json"
(cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js") | grep -qx "onetaskgraph $version" || { echo "launcher did not execute the carrier; next: inspect package resolution" >&2; exit 1; }
set +e
(cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js" --exit 7)
launcher_status=$?
set -e
[[ $launcher_status -eq 7 ]] || { echo "launcher did not propagate status 7; next: inspect spawn result handling" >&2; exit 1; }
mv "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/onetaskgraph" "$tmp/missing-binary"
if (cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js") 2>"$tmp/error"; then echo "launcher accepted a missing carrier binary; next: inspect spawn errors" >&2; exit 1; fi
mv "$tmp/missing-binary" "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/onetaskgraph"
mv "$tmp/node_modules/@onetaskgraph/cli-${node_platform}" "$tmp/missing-carrier"
if (cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js") 2>"$tmp/error"; then echo "launcher accepted a missing carrier; next: inspect package resolution" >&2; exit 1; fi
grep -q 'is not installed' "$tmp/error" || { echo "missing-carrier failure omitted recovery guidance; next: inspect launcher diagnostics" >&2; exit 1; }
