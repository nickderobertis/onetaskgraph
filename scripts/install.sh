#!/bin/sh
set -eu

repo=nickderobertis/onetaskgraph
canonical="https://github.com/$repo/releases/download"
version=${ONETASKGRAPH_VERSION:-}
install_dir=${ONETASKGRAPH_INSTALL_DIR:-"$HOME/.local/bin"}
release_base=${ONETASKGRAPH_RELEASE_BASE_URL:-$canonical}
checksum_base=${ONETASKGRAPH_CHECKSUM_BASE_URL:-$canonical}

die() { printf 'error: %s\n' "$*" >&2; exit 1; }
origin() { printf '%s\n' "$1" | sed -E 's#^([a-zA-Z][a-zA-Z0-9+.-]*://[^/]+).*#\1#'; }
download() { case "$1" in file://*) cp "${1#file://}" "$2";; *) curl -fsSL "$1" -o "$2";; esac; }

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version) version=$2; shift 2;;
    --to) install_dir=$2; shift 2;;
    --base-url) release_base=$2; shift 2;;
    *) die "unknown option: $1";;
  esac
done
[ -n "$version" ] || version=$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest" | sed -n 's/.*"tag_name":[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)
case "$version" in v[0-9]*.[0-9]*.[0-9]*) ;; *) die "unsupported release tag: $version";; esac

case "$(uname -s):$(uname -m)" in
  Linux:x86_64|Linux:amd64) target=x86_64-unknown-linux-gnu; ext=tar.gz; binary=onetaskgraph;;
  Linux:aarch64|Linux:arm64) target=aarch64-unknown-linux-gnu; ext=tar.gz; binary=onetaskgraph;;
  Darwin:x86_64) target=x86_64-apple-darwin; ext=tar.gz; binary=onetaskgraph;;
  Darwin:arm64|Darwin:aarch64) target=aarch64-apple-darwin; ext=tar.gz; binary=onetaskgraph;;
  MINGW*:x86_64|MSYS*:x86_64|CYGWIN*:x86_64) target=x86_64-pc-windows-msvc; ext=zip; binary=onetaskgraph.exe;;
  *) die "no prebuilt binary for $(uname -s) $(uname -m)";;
esac

name="onetaskgraph-${version}-${target}.${ext}"
archive_url="${release_base%/}/${version}/${name}"
checksum_url="${checksum_base%/}/${version}/${name}.sha256"
if [ "$(origin "$release_base")" = "$(origin "$checksum_base")" ] && [ "$(origin "$release_base")" != "$(origin "$canonical")" ]; then
  die "checksum shares the mirror's origin; refusing a mirror-controlled trust root"
fi
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT HUP INT TERM
download "$archive_url" "$tmp/$name" || die "download failed: $archive_url"
download "$checksum_url" "$tmp/$name.sha256" || die "checksum download failed: $checksum_url"
expected=$(awk '{print $1}' "$tmp/$name.sha256")
actual=$(if command -v sha256sum >/dev/null 2>&1; then sha256sum "$tmp/$name" | awk '{print $1}'; else shasum -a 256 "$tmp/$name" | awk '{print $1}'; fi)
[ "$actual" = "$expected" ] || die "checksum mismatch for $name"
mkdir -p "$tmp/unpack" "$install_dir"
case "$ext" in tar.gz) tar -xzf "$tmp/$name" -C "$tmp/unpack";; zip) unzip -q "$tmp/$name" -d "$tmp/unpack";; esac
install -m 755 "$tmp/unpack/$binary" "$install_dir/$binary"
printf 'installed %s to %s\n' "$version" "$install_dir/$binary"
