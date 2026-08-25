#!/bin/sh
set -eu

repo=nickderobertis/onetaskgraph
canonical="https://github.com/$repo/releases/download"
version=${ONETASKGRAPH_VERSION:-}
install_dir=${ONETASKGRAPH_INSTALL_DIR:-"$HOME/.local/bin"}
release_base=${ONETASKGRAPH_RELEASE_BASE_URL:-$canonical}
checksum_base=${ONETASKGRAPH_CHECKSUM_BASE_URL:-$canonical}

die() { code=$1; shift; printf 'error: %s\nnext: run scripts/install.sh --help or choose another documented install method\n' "$*" >&2; exit "$code"; }
origin() { printf '%s\n' "$1" | sed -E 's#^([a-zA-Z][a-zA-Z0-9+.-]*://[^/]+).*#\1#'; }
download() { case "$1" in file://*) cp "${1#file://}" "$2";; *) curl -fsSL "$1" -o "$2";; esac; }

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version|--to|--archive-base-url) [ "$#" -ge 2 ] || die 64 "$1 requires a value";;
  esac
  case "$1" in
    --version) version=$2; shift 2;;
    --to) install_dir=$2; shift 2;;
    --archive-base-url) release_base=$2; shift 2;;
    --help) printf 'usage: install.sh [--version vX.Y.Z] [--to DIR] [--archive-base-url URL]\nexit codes: 64 usage; 65 integrity; 69 unavailable platform/download; 74 installation failure\n'; exit 0;;
    *) die 64 "unknown option: $1";;
  esac
done
if [ -z "$version" ]; then
  body=$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest") || die 69 "could not resolve the latest GitHub Release"
  version=$(printf '%s\n' "$body" | sed -n 's/.*"tag_name":[[:space:]]*"\([^"]*\)".*/\1/p' | head -n1)
fi
printf '%s\n' "$version" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)?$' || die 64 "unsupported release tag: $version"

case "$(uname -s):$(uname -m)" in
  Linux:x86_64|Linux:amd64) target=x86_64-unknown-linux-gnu; ext=tar.gz; binary=onetaskgraph;;
  Linux:aarch64|Linux:arm64) target=aarch64-unknown-linux-gnu; ext=tar.gz; binary=onetaskgraph;;
  Darwin:x86_64) target=x86_64-apple-darwin; ext=tar.gz; binary=onetaskgraph;;
  Darwin:arm64|Darwin:aarch64) target=aarch64-apple-darwin; ext=tar.gz; binary=onetaskgraph;;
  MINGW*:x86_64|MSYS*:x86_64|CYGWIN*:x86_64) target=x86_64-pc-windows-msvc; ext=zip; binary=onetaskgraph.exe;;
  *) die 69 "no prebuilt binary for $(uname -s) $(uname -m)";;
esac

name="onetaskgraph-${version}-${target}.${ext}"
archive_url="${release_base%/}/${version}/${name}"
checksum_url="${checksum_base%/}/${version}/${name}.sha256"
if [ "$(origin "$release_base")" = "$(origin "$checksum_base")" ] && [ "$(origin "$release_base")" != "$(origin "$canonical")" ]; then
  die 65 "checksum shares the mirror's origin; refusing a mirror-controlled trust root"
fi
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT HUP INT TERM
download "$archive_url" "$tmp/$name" || die 69 "download failed: $archive_url"
download "$checksum_url" "$tmp/$name.sha256" || die 69 "checksum download failed: $checksum_url"
records=$(awk 'NF { count++ } END { print count+0 }' "$tmp/$name.sha256")
[ "$records" -eq 1 ] || die 65 "checksum file must contain exactly one SHA-256 record"
expected=$(awk 'NF {print $1}' "$tmp/$name.sha256")
printf '%s\n' "$expected" | grep -Eq '^[0-9A-Fa-f]{64}$' || die 65 "checksum file does not contain one SHA-256 digest"
if command -v sha256sum >/dev/null 2>&1; then actual=$(sha256sum "$tmp/$name" | awk '{print $1}'); elif command -v shasum >/dev/null 2>&1; then actual=$(shasum -a 256 "$tmp/$name" | awk '{print $1}'); else die 69 "no SHA-256 implementation is installed"; fi
[ "$actual" = "$expected" ] || die 65 "checksum mismatch for $name"
mkdir -p "$tmp/unpack" "$install_dir"
case "$ext" in
  tar.gz) members=$(tar -tzf "$tmp/$name") || die 65 "archive is unreadable: $name";;
  zip) members=$(unzip -Z1 "$tmp/$name") || die 65 "archive is unreadable: $name";;
esac
printf '%s\n' "$members" | awk '/^\// || /^[A-Za-z]:/ || /(^|\/)\.\.($|\/)/ { bad=1 } END { exit bad }' || die 65 "archive contains an unsafe member path"
case "$ext" in tar.gz) tar -xzf "$tmp/$name" -C "$tmp/unpack" || die 74 "could not extract $name";; zip) unzip -q "$tmp/$name" -d "$tmp/unpack" || die 74 "could not extract $name";; esac
[ -f "$tmp/unpack/$binary" ] && [ ! -L "$tmp/unpack/$binary" ] || die 65 "archive binary is not a regular file"
install -m 755 "$tmp/unpack/$binary" "$install_dir/$binary" || die 74 "could not install into $install_dir"
printf 'installed %s to %s\n' "$version" "$install_dir/$binary"
