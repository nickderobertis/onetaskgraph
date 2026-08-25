#!/usr/bin/env bash
set -euo pipefail
trap 'echo "distribution setup failed; next: rerun scripts/test-distribution.sh and inspect the command immediately above this diagnostic" >&2' ERR
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$root/crates/onetaskgraph/Cargo.toml" | head -n1)
tag="v$version"
mkdir -p "$tmp/releases/$tag" "$tmp/canonical/$tag" "$tmp/bin"
RUSTFLAGS='-D warnings' cargo build --manifest-path "$root/Cargo.toml" --locked -p onetaskgraph --quiet
target=x86_64-unknown-linux-gnu
ext=tar.gz
binary=onetaskgraph
case "$(uname -s):$(uname -m)" in
  Darwin:x86_64) target=x86_64-apple-darwin;;
  Darwin:arm64|Darwin:aarch64) target=aarch64-apple-darwin;;
  Linux:aarch64|Linux:arm64) target=aarch64-unknown-linux-gnu;;
  MINGW*:x86_64|MSYS*:x86_64|CYGWIN*:x86_64) target=x86_64-pc-windows-msvc; ext=zip; binary=onetaskgraph.exe;;
esac
name="onetaskgraph-${tag}-${target}.${ext}"
if [[ $ext == zip ]]; then (cd "$root/target/debug" && 7z a "$tmp/releases/$tag/$name" "$binary" >/dev/null); else tar -czf "$tmp/releases/$tag/$name" -C "$root/target/debug" "$binary"; fi
if command -v sha256sum >/dev/null; then sha256sum "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"; else shasum -a 256 "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"; fi
ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" >/dev/null
"$tmp/bin/$binary" --help | grep -q 'Usage:' || { echo "installed command did not render help; next: inspect the locally archived binary" >&2; exit 1; }
ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" "$root/scripts/install.sh" --version "$tag" --to "$tmp/bin" --archive-base-url "file://$tmp/releases" >/dev/null
"$root/scripts/install.sh" --help | grep -q 'exit codes:' || { echo "installer help omitted exit codes; next: update its public usage contract" >&2; exit 1; }
if "$root/scripts/install.sh" --unknown 2>"$tmp/error"; then echo "unknown option was accepted; next: inspect argument parsing" >&2; exit 1; fi
printf x >> "$tmp/releases/$tag/$name"
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "tampered archive installed; next: inspect checksum verification" >&2; exit 1; fi
grep -q 'checksum mismatch' "$tmp/error" || { echo "tamper failure omitted checksum mismatch; next: inspect installer diagnostics" >&2; exit 1; }
printf 'not an archive' > "$tmp/releases/$tag/$name"
if command -v sha256sum >/dev/null; then sha256sum "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"; else shasum -a 256 "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"; fi
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "unreadable archive installed; next: inspect archive validation" >&2; exit 1; fi
grep -q 'archive is unreadable' "$tmp/error" || { echo "archive failure omitted its reason; next: inspect archive validation" >&2; exit 1; }
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/missing" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "missing archive installed; next: inspect download failure handling" >&2; exit 1; fi
grep -q 'download failed' "$tmp/error" || { echo "download failure omitted its reason; next: inspect download diagnostics" >&2; exit 1; }
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL=https://mirror.example/releases ONETASKGRAPH_CHECKSUM_BASE_URL=https://mirror.example/checks "$root/scripts/install.sh" 2>"$tmp/error"; then echo "mirror-controlled checksum was accepted; next: inspect origin comparison" >&2; exit 1; fi
grep -q "checksum shares the mirror's origin" "$tmp/error" || { echo "mirror rejection omitted its reason; next: inspect installer diagnostics" >&2; exit 1; }
if "$root/scripts/install.sh" --version 2>"$tmp/error"; then echo "missing option value was accepted; next: inspect installer argument parsing" >&2; exit 1; fi
grep -q 'requires a value' "$tmp/error" || { echo "missing-value failure omitted its reason; next: inspect argument diagnostics" >&2; exit 1; }
if ONETASKGRAPH_VERSION="${tag}junk" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "malformed tag was accepted; next: inspect tag validation" >&2; exit 1; fi
node_platform=$(node -p '`${process.platform}-${process.arch}`')
mkdir -p "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin"
cp "$root/target/debug/$binary" "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/$binary"
printf '{"name":"@onetaskgraph/cli-%s"}\n' "$node_platform" > "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/package.json"
(cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js" --help) | grep -q 'Usage:' || { echo "launcher did not execute the carrier; next: inspect package resolution" >&2; exit 1; }
set +e
(cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js" --definitely-invalid >/dev/null 2>&1)
launcher_status=$?
set -e
[[ $launcher_status -eq 2 ]] || { echo "launcher did not propagate command status 2; next: inspect spawn result handling" >&2; exit 1; }
mv "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/$binary" "$tmp/missing-binary"
if (cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js") 2>"$tmp/error"; then echo "launcher accepted a missing carrier binary; next: inspect spawn errors" >&2; exit 1; fi
mv "$tmp/missing-binary" "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/$binary"
mv "$tmp/node_modules/@onetaskgraph/cli-${node_platform}" "$tmp/missing-carrier"
if (cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js") 2>"$tmp/error"; then echo "launcher accepted a missing carrier; next: inspect package resolution" >&2; exit 1; fi
grep -q 'is not installed' "$tmp/error" || { echo "missing-carrier failure omitted recovery guidance; next: inspect launcher diagnostics" >&2; exit 1; }
