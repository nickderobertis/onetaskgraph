#!/usr/bin/env bash
set -euo pipefail
report_failure() {
  echo "distribution setup failed at line $2 (exit $1): $3; next: rerun scripts/test-distribution.sh and fix that command" >&2
}
trap 'report_failure "$?" "$LINENO" "$BASH_COMMAND"' ERR
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tmp=$(mktemp -d)
python_bin=$(command -v python3 || command -v python || true)
[[ -n $python_bin ]] || { echo "distribution test requires Python 3; next: install python3 and rerun scripts/test-distribution.sh" >&2; exit 1; }
"$python_bin" -c 'import sys; raise SystemExit(sys.version_info < (3, 8))' || { echo "distribution test requires Python 3.8 or newer; next: install a supported python3 and rerun scripts/test-distribution.sh" >&2; exit 1; }
cleanup() {
  if [[ -n ${http_server_pid:-} ]]; then kill "$http_server_pid" 2>/dev/null || true; wait "$http_server_pid" 2>/dev/null || true; fi
  rm -rf "$tmp"
}
trap cleanup EXIT
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$root/crates/onetaskgraph/Cargo.toml" | head -n1)
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$ ]] || {
  echo "distribution test found an invalid binary version; next: restore crates/onetaskgraph/Cargo.toml to an X.Y.Z version" >&2
  exit 1
}
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
install_output=$(ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh")
printf '%s\n' "$install_output" | grep -q "installed $tag to" || { echo "installer success omitted its destination; next: inspect the success contract" >&2; exit 1; }
"$tmp/bin/$binary" --help | grep -q 'Usage:' || { echo "installed command did not render help; next: inspect the locally archived binary" >&2; exit 1; }
ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" "$root/scripts/install.sh" --version "$tag" --to "$tmp/bin" --archive-base-url "file://$tmp/releases" >/dev/null
"$root/scripts/install.sh" --help | grep -q 'exit codes:' || { echo "installer help omitted exit codes; next: update its public usage contract" >&2; exit 1; }
if "$root/scripts/install.sh" --unknown 2>"$tmp/error"; then unknown_option_status=0; else unknown_option_status=$?; fi
[[ $unknown_option_status -eq 64 ]] || { echo "unknown option exited $unknown_option_status, expected 64; next: inspect argument parsing" >&2; exit 1; }
grep -q 'unknown option: --unknown' "$tmp/error" || { cat "$tmp/error" >&2; echo "unknown-option failure omitted its reason; next: inspect argument diagnostics" >&2; exit 1; }
printf x >> "$tmp/releases/$tag/$name"
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "tampered archive installed; next: inspect checksum verification" >&2; exit 1; fi
grep -q 'checksum mismatch' "$tmp/error" || { cat "$tmp/error" >&2; echo "tamper failure omitted checksum mismatch; next: inspect installer diagnostics" >&2; exit 1; }
printf 'not an archive' > "$tmp/releases/$tag/$name"
if command -v sha256sum >/dev/null; then sha256sum "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"; else shasum -a 256 "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"; fi
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "unreadable archive installed; next: inspect archive validation" >&2; exit 1; fi
grep -q 'archive is unreadable' "$tmp/error" || { cat "$tmp/error" >&2; echo "archive failure omitted its reason; next: inspect archive validation" >&2; exit 1; }
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/missing" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "missing archive installed; next: inspect download failure handling" >&2; exit 1; fi
grep -q 'download failed' "$tmp/error" || { cat "$tmp/error" >&2; echo "download failure omitted its reason; next: inspect download diagnostics" >&2; exit 1; }
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/missing" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "missing checksum accepted; next: inspect checksum download handling" >&2; exit 1; fi
grep -q 'checksum download failed' "$tmp/error" || { cat "$tmp/error" >&2; echo "checksum download failure omitted its reason; next: inspect download diagnostics" >&2; exit 1; }
printf 'not-a-digest\n' > "$tmp/canonical/$tag/$name.sha256"
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "malformed checksum accepted; next: inspect checksum parsing" >&2; exit 1; fi
grep -q 'does not contain one SHA-256 digest' "$tmp/error" || { cat "$tmp/error" >&2; echo "malformed checksum failure omitted its reason; next: inspect checksum diagnostics" >&2; exit 1; }
printf '%064d first\n%064d second\n' 0 1 > "$tmp/canonical/$tag/$name.sha256"
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "multiple checksum records accepted; next: inspect checksum cardinality" >&2; exit 1; fi
grep -q 'exactly one SHA-256 record' "$tmp/error" || { cat "$tmp/error" >&2; echo "multi-record checksum failure omitted its reason; next: inspect checksum diagnostics" >&2; exit 1; }
"$python_bin" - "$tmp/releases/$tag/$name" "$ext" <<'PY'
import io
import sys
import tarfile
import zipfile

path, extension = sys.argv[1:]
if extension == "zip":
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("..\\escape", b"unsafe")
else:
    with tarfile.open(path, "w:gz") as archive:
        member = tarfile.TarInfo("../escape")
        member.size = 6
        archive.addfile(member, io.BytesIO(b"unsafe"))
PY
if command -v sha256sum >/dev/null; then sha256sum "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"; else shasum -a 256 "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"; fi
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "unsafe archive member installed; next: inspect member validation" >&2; exit 1; fi
grep -q 'unsafe member path' "$tmp/error" || { cat "$tmp/error" >&2; echo "unsafe-member failure omitted its reason; next: inspect archive validation" >&2; exit 1; }
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="ftp://invalid/releases" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "unsupported download scheme was accepted; next: inspect source validation" >&2; exit 1; fi
grep -q 'download base must use' "$tmp/error" || { cat "$tmp/error" >&2; echo "unsupported-scheme failure omitted its reason; next: inspect source diagnostics" >&2; exit 1; }
rm "$tmp/releases/$tag/$name"
if [[ $ext == zip ]]; then (cd "$root/target/debug" && 7z a "$tmp/releases/$tag/$name" "$binary" >/dev/null); else tar -czf "$tmp/releases/$tag/$name" -C "$root/target/debug" "$binary"; fi
if command -v sha256sum >/dev/null; then sha256sum "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"; else shasum -a 256 "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"; fi
"$python_bin" - "$tmp" "$tmp/http-port" >"$tmp/http.log" 2>&1 <<'PY' &
import http.server
import os
import sys

root, port_file = sys.argv[1:]
os.chdir(root)
server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), http.server.SimpleHTTPRequestHandler)
with open(port_file, "w", encoding="utf-8") as stream:
    stream.write(str(server.server_port))
server.serve_forever()
PY
http_server_pid=$!
http_server_status=
for _ in {1..600}; do
  [[ -s "$tmp/http-port" ]] && break
  if ! kill -0 "$http_server_pid" 2>/dev/null; then
    if wait "$http_server_pid"; then http_server_status=0; else http_server_status=$?; fi
    http_server_pid=
    break
  fi
  sleep 0.05
done
if [[ ! -s "$tmp/http-port" ]]; then
  if [[ -n $http_server_status ]]; then
    echo "local HTTP release server exited with status $http_server_status before starting; server output follows:" >&2
  else
    echo "local HTTP release server was still running after 30 seconds; server output follows:" >&2
  fi
  sed 's/^/  /' "$tmp/http.log" >&2
  echo "next: inspect the distribution test server" >&2
  exit 1
fi
http_port=$(<"$tmp/http-port")
ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="http://127.0.0.1:$http_port/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" >/dev/null
kill "$http_server_pid"
wait "$http_server_pid" 2>/dev/null || true
http_server_pid=
mkdir -p "$tmp/shims"
printf '#!/bin/sh\necho '\''{"tag_name":"%s"}'\''\n' "$tag" > "$tmp/shims/curl"
chmod +x "$tmp/shims/curl"
PATH="$tmp/shims:$PATH" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" >/dev/null
printf '#!/bin/sh\nexit 1\n' > "$tmp/shims/curl"
if PATH="$tmp/shims:$PATH" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "latest-release lookup failure was accepted; next: inspect release resolution" >&2; exit 1; fi
grep -q 'could not resolve the latest GitHub Release' "$tmp/error" || { cat "$tmp/error" >&2; echo "latest-release failure omitted its reason; next: inspect release diagnostics" >&2; exit 1; }
printf '#!/bin/sh\nprintf '\''Unsupported\\n'\''\n' > "$tmp/shims/uname"
chmod +x "$tmp/shims/uname"
if PATH="$tmp/shims:$PATH" ONETASKGRAPH_VERSION="$tag" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "unsupported installer platform was accepted; next: inspect platform selection" >&2; exit 1; fi
grep -q 'no prebuilt binary' "$tmp/error" || { cat "$tmp/error" >&2; echo "unsupported-installer-platform failure omitted its reason; next: inspect platform diagnostics" >&2; exit 1; }
rm "$tmp/shims/uname" "$tmp/shims/curl"
printf 'not a directory' > "$tmp/not-a-directory"
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/not-a-directory/child" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "uncreatable installation directory was accepted; next: inspect directory creation" >&2; exit 1; fi
grep -q 'could not create the installation directory' "$tmp/error" || { cat "$tmp/error" >&2; echo "directory-creation failure omitted its reason; next: inspect filesystem diagnostics" >&2; exit 1; }
"$python_bin" - "$tmp/releases/$tag/$name" "$ext" <<'PY'
import sys
import tarfile
import zipfile

path, extension = sys.argv[1:]
if extension == "zip":
    with zipfile.ZipFile(path, "w") as archive:
        member = zipfile.ZipInfo("linked")
        # ZipInfo defaults to the host OS. Declare Unix explicitly so Windows readers
        # interpret the high external-attribute bits below as a symlink mode too.
        member.create_system = 3
        member.external_attr = 0o120777 << 16
        archive.writestr(member, "onetaskgraph")
else:
    with tarfile.open(path, "w:gz") as archive:
        member = tarfile.TarInfo("linked")
        member.type = tarfile.SYMTYPE
        member.linkname = "onetaskgraph"
        archive.addfile(member)
PY
if command -v sha256sum >/dev/null; then sha256sum "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"; else shasum -a 256 "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"; fi
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "archive link entry was accepted; next: inspect link validation" >&2; exit 1; fi
grep -q 'archive contains a link entry' "$tmp/error" || { cat "$tmp/error" >&2; echo "link-entry failure omitted its reason; next: inspect archive diagnostics" >&2; exit 1; }
rm "$tmp/releases/$tag/$name"
if [[ $ext == zip ]]; then (cd "$tmp" && 7z a "$tmp/releases/$tag/$name" error >/dev/null); else tar -czf "$tmp/releases/$tag/$name" -C "$tmp" error; fi
if command -v sha256sum >/dev/null; then sha256sum "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"; else shasum -a 256 "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"; fi
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "archive without binary was accepted; next: inspect binary validation" >&2; exit 1; fi
grep -q 'archive binary is not a regular file' "$tmp/error" || { cat "$tmp/error" >&2; echo "missing-archive-binary failure omitted its reason; next: inspect archive diagnostics" >&2; exit 1; }
rm "$tmp/releases/$tag/$name"
if [[ $ext == zip ]]; then (cd "$root/target/debug" && 7z a "$tmp/releases/$tag/$name" "$binary" >/dev/null); else tar -czf "$tmp/releases/$tag/$name" -C "$root/target/debug" "$binary"; fi
if command -v sha256sum >/dev/null; then sha256sum "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"; else shasum -a 256 "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"; fi
mkdir "$tmp/no-hash-path"
for utility in awk cp dirname grep install mkdir mktemp rm sed tar uname; do
  utility_path=$(command -v "$utility")
  printf '#!/bin/sh\nexec "%s" "$@"\n' "$utility_path" > "$tmp/no-hash-path/$utility"
  chmod +x "$tmp/no-hash-path/$utility"
done
if PATH="$tmp/no-hash-path" ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "installer accepted a system without SHA-256 tooling; next: inspect hash-tool detection" >&2; exit 1; fi
grep -q 'no SHA-256 implementation is installed' "$tmp/error" || { cat "$tmp/error" >&2; echo "missing-hash-tool failure omitted its reason; next: inspect integrity diagnostics" >&2; exit 1; }
printf '#!/bin/sh\nexit 1\n' > "$tmp/shims/mktemp"
chmod +x "$tmp/shims/mktemp"
if PATH="$tmp/shims:$PATH" ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "temporary-directory failure was accepted; next: inspect temporary setup" >&2; exit 1; fi
grep -q 'could not create a temporary directory' "$tmp/error" || { cat "$tmp/error" >&2; echo "temporary-directory failure omitted its reason; next: inspect filesystem diagnostics" >&2; exit 1; }
rm "$tmp/shims/mktemp"
if [[ $ext == tar.gz ]]; then
  real_tar=$(command -v tar)
  printf '#!/bin/sh\nif [ "$1" = -tvzf ]; then exit 1; fi\nexec %s "$@"\n' "$real_tar" > "$tmp/shims/tar"
  chmod +x "$tmp/shims/tar"
  if PATH="$tmp/shims:$PATH" ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "unreadable archive metadata was accepted; next: inspect metadata validation" >&2; exit 1; fi
  grep -q 'archive metadata is unreadable' "$tmp/error" || { cat "$tmp/error" >&2; echo "metadata failure omitted its reason; next: inspect archive diagnostics" >&2; exit 1; }
  printf '#!/bin/sh\nif [ "$1" = -xzf ]; then exit 1; fi\nexec %s "$@"\n' "$real_tar" > "$tmp/shims/tar"
  if PATH="$tmp/shims:$PATH" ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "archive extraction failure was accepted; next: inspect extraction handling" >&2; exit 1; fi
  grep -q 'could not extract' "$tmp/error" || { cat "$tmp/error" >&2; echo "extraction failure omitted its reason; next: inspect archive diagnostics" >&2; exit 1; }
  rm "$tmp/shims/tar"
fi
printf '#!/bin/sh\nexit 1\n' > "$tmp/shims/install"
chmod +x "$tmp/shims/install"
if PATH="$tmp/shims:$PATH" ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "final installation failure was accepted; next: inspect executable installation" >&2; exit 1; fi
grep -q 'could not install into' "$tmp/error" || { cat "$tmp/error" >&2; echo "installation-copy failure omitted its reason; next: inspect filesystem diagnostics" >&2; exit 1; }
rm "$tmp/shims/install"
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL=https://mirror.example/releases ONETASKGRAPH_CHECKSUM_BASE_URL=https://mirror.example/checks "$root/scripts/install.sh" 2>"$tmp/error"; then echo "mirror-controlled checksum was accepted; next: inspect origin comparison" >&2; exit 1; fi
grep -q "checksum shares the mirror's origin" "$tmp/error" || { cat "$tmp/error" >&2; echo "mirror rejection omitted its reason; next: inspect installer diagnostics" >&2; exit 1; }
if "$root/scripts/install.sh" --version 2>"$tmp/error"; then missing_value_status=0; else missing_value_status=$?; fi
[[ $missing_value_status -eq 64 ]] || { echo "missing option value exited $missing_value_status, expected 64; next: inspect argument parsing" >&2; exit 1; }
grep -q 'requires a value' "$tmp/error" || { cat "$tmp/error" >&2; echo "missing-value failure omitted its reason; next: inspect argument diagnostics" >&2; exit 1; }
if "$root/scripts/install.sh" --to '' 2>"$tmp/error"; then empty_destination_status=0; else empty_destination_status=$?; fi
[[ $empty_destination_status -eq 64 ]] || { echo "empty installation destination exited $empty_destination_status, expected 64; next: inspect destination validation" >&2; exit 1; }
grep -q 'installation directory must not be empty' "$tmp/error" || { cat "$tmp/error" >&2; echo "empty-destination failure omitted its reason; next: inspect destination diagnostics" >&2; exit 1; }
if "$root/scripts/install.sh" --to '-unsafe' 2>"$tmp/error"; then leading_dash_status=0; else leading_dash_status=$?; fi
[[ $leading_dash_status -eq 64 ]] || { echo "leading-dash installation destination exited $leading_dash_status, expected 64; next: inspect destination validation" >&2; exit 1; }
grep -q "installation directory must not begin with '-'" "$tmp/error" || { cat "$tmp/error" >&2; echo "leading-dash destination failure omitted its reason; next: inspect destination diagnostics" >&2; exit 1; }
if ONETASKGRAPH_VERSION="${tag}junk" "$root/scripts/install.sh" 2>"$tmp/error"; then malformed_tag_status=0; else malformed_tag_status=$?; fi
[[ $malformed_tag_status -eq 64 ]] || { echo "malformed tag exited $malformed_tag_status, expected 64; next: inspect tag validation" >&2; exit 1; }
grep -q "unsupported release tag: ${tag}junk" "$tmp/error" || { cat "$tmp/error" >&2; echo "malformed-tag failure omitted its reason; next: inspect tag diagnostics" >&2; exit 1; }
if ONETASKGRAPH_VERSION="${tag}+build+again" "$root/scripts/install.sh" 2>"$tmp/error"; then repeated_metadata_status=0; else repeated_metadata_status=$?; fi
[[ $repeated_metadata_status -eq 64 ]] || { echo "repeated metadata separator exited $repeated_metadata_status, expected 64; next: inspect tag validation" >&2; exit 1; }
grep -q "unsupported release tag: ${tag}+build+again" "$tmp/error" || { cat "$tmp/error" >&2; echo "repeated-metadata failure omitted its reason; next: inspect tag diagnostics" >&2; exit 1; }
node_platform=$(node -p '`${process.platform}-${process.arch}`')
mkdir -p "$tmp/npm-carrier/bin" "$tmp/npm-packages" "$tmp/npm-install"
cp "$root/npm/platforms/$node_platform/package.json" "$tmp/npm-carrier/package.json"
cp "$root/target/debug/$binary" "$tmp/npm-carrier/bin/$binary"
carrier_package=$(npm pack "$tmp/npm-carrier" --silent --pack-destination "$tmp/npm-packages")
launcher_package=$(npm pack "$root/npm/cli" --silent --pack-destination "$tmp/npm-packages")
printf '{"private":true}\n' > "$tmp/npm-install/package.json"
(cd "$tmp/npm-install" && npm install --silent --offline --ignore-scripts "$tmp/npm-packages/$carrier_package" "$tmp/npm-packages/$launcher_package" >/dev/null)
"$tmp/npm-install/node_modules/.bin/onetaskgraph" --help | grep -q 'Usage:' || { echo "packed npm command did not render help; next: inspect the launcher and carrier package contents" >&2; exit 1; }
mkdir -p "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin"
cp "$root/target/debug/$binary" "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/$binary"
printf '{"name":"@onetaskgraph/cli-%s"}\n' "$node_platform" > "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/package.json"
(cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js" --help) | grep -q 'Usage:' || { echo "launcher did not execute the carrier; next: inspect package resolution" >&2; exit 1; }
printf 'not json\n' > "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/package.json"
if (cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js") 2>"$tmp/error"; then malformed_carrier_status=0; else malformed_carrier_status=$?; fi
[[ $malformed_carrier_status -eq 69 ]] || { echo "malformed carrier exited $malformed_carrier_status, expected 69; next: inspect manifest validation" >&2; exit 1; }
grep -q 'invalid @onetaskgraph/cli-' "$tmp/error" || { cat "$tmp/error" >&2; echo "malformed-carrier failure omitted its reason; next: inspect launcher diagnostics" >&2; exit 1; }
printf '{"name":"wrong-carrier"}\n' > "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/package.json"
if (cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js") 2>"$tmp/error"; then wrong_carrier_status=0; else wrong_carrier_status=$?; fi
[[ $wrong_carrier_status -eq 69 ]] || { echo "wrong carrier identity exited $wrong_carrier_status, expected 69; next: inspect manifest validation" >&2; exit 1; }
grep -q 'identifies itself as wrong-carrier' "$tmp/error" || { cat "$tmp/error" >&2; echo "wrong-carrier failure omitted its reason; next: inspect launcher diagnostics" >&2; exit 1; }
printf '{"name":"@onetaskgraph/cli-%s"}\n' "$node_platform" > "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/package.json"
mv "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin" "$tmp/real-carrier-bin"
node -e 'require("node:fs").symlinkSync(process.argv[1],process.argv[2],process.platform === "win32" ? "junction" : "dir")' "$tmp/real-carrier-bin" "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin"
if (cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js") 2>"$tmp/error"; then escaping_carrier_status=0; else escaping_carrier_status=$?; fi
[[ $escaping_carrier_status -eq 69 ]] || { echo "escaping carrier exited $escaping_carrier_status, expected 69; next: inspect carrier containment" >&2; exit 1; }
grep -q 'carrier binary escapes its package' "$tmp/error" || { cat "$tmp/error" >&2; echo "escaping-carrier failure omitted its reason; next: inspect launcher diagnostics" >&2; exit 1; }
rm "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin"
mv "$tmp/real-carrier-bin" "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin"
if [[ $node_platform != win32-* ]]; then
  mv "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/$binary" "$tmp/real-carrier"
  printf '#!/bin/sh\nkill -TERM $$\n' > "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/$binary"
  chmod +x "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/$binary"
  if (cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js") 2>"$tmp/error"; then signal_status=0; else signal_status=$?; fi
  [[ $signal_status -eq 70 ]] || { echo "signaled carrier exited $signal_status, expected 70; next: inspect signal handling" >&2; exit 1; }
  grep -q 'carrier terminated by' "$tmp/error" || { cat "$tmp/error" >&2; echo "signal failure omitted its reason; next: inspect launcher diagnostics" >&2; exit 1; }
  mv "$tmp/real-carrier" "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/$binary"
fi
if (cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js" --definitely-invalid >/dev/null 2>&1); then launcher_status=0; else launcher_status=$?; fi
[[ $launcher_status -eq 2 ]] || { echo "launcher did not propagate command status 2; next: inspect spawn result handling" >&2; exit 1; }
if [[ $node_platform != win32-* ]]; then
  chmod -x "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/$binary"
  if (cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js") 2>"$tmp/error"; then non_executable_status=0; else non_executable_status=$?; fi
  [[ $non_executable_status -eq 69 ]] || { echo "non-executable carrier exited $non_executable_status, expected 69; next: inspect spawn errors" >&2; exit 1; }
  grep -q 'reinstall the platform package' "$tmp/error" || { cat "$tmp/error" >&2; echo "spawn failure omitted recovery guidance; next: inspect launcher diagnostics" >&2; exit 1; }
  chmod +x "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/$binary"
fi
mv "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/$binary" "$tmp/missing-binary"
if (cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js") 2>"$tmp/error"; then missing_binary_status=0; else missing_binary_status=$?; fi
[[ $missing_binary_status -eq 69 ]] || { echo "missing carrier binary exited $missing_binary_status, expected 69; next: inspect spawn errors" >&2; exit 1; }
grep -q 'reinstall the platform package' "$tmp/error" || { cat "$tmp/error" >&2; echo "missing-binary failure omitted recovery guidance; next: inspect spawn errors" >&2; exit 1; }
mv "$tmp/missing-binary" "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/$binary"
mv "$tmp/node_modules/@onetaskgraph/cli-${node_platform}" "$tmp/missing-carrier"
if (cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js") 2>"$tmp/error"; then missing_carrier_status=0; else missing_carrier_status=$?; fi
[[ $missing_carrier_status -eq 69 ]] || { echo "missing carrier exited $missing_carrier_status, expected 69; next: inspect package resolution" >&2; exit 1; }
grep -q 'is not installed' "$tmp/error" || { cat "$tmp/error" >&2; echo "missing-carrier failure omitted recovery guidance; next: inspect launcher diagnostics" >&2; exit 1; }
if node -e 'const Module=require("node:module"),original=Module._resolveFilename; Module._resolveFilename=function(request,...args){if(request.startsWith("@onetaskgraph/cli-")){const error=new Error("permission denied"); error.code="EACCES"; throw error;} return original.call(this,request,...args);}; require(process.argv[1]);' "$root/npm/cli/bin/onetaskgraph.js" 2>"$tmp/error"; then resolution_status=0; else resolution_status=$?; fi
[[ $resolution_status -eq 69 ]] || { echo "carrier resolution error exited $resolution_status, expected 69; next: inspect package resolution" >&2; exit 1; }
grep -q 'permission denied; reinstall the platform package' "$tmp/error" || { cat "$tmp/error" >&2; echo "carrier-resolution failure omitted its reason; next: inspect launcher diagnostics" >&2; exit 1; }
if node -e 'Object.defineProperty(process,"platform",{value:"unsupported"}); require(process.argv[1])' "$root/npm/cli/bin/onetaskgraph.js" 2>"$tmp/error"; then unsupported_status=0; else unsupported_status=$?; fi
[[ $unsupported_status -eq 64 ]] || { echo "unsupported launcher platform exited $unsupported_status, expected 64; next: inspect platform validation" >&2; exit 1; }
grep -q 'unsupported platform' "$tmp/error" || { cat "$tmp/error" >&2; echo "unsupported-platform failure omitted its reason; next: inspect launcher diagnostics" >&2; exit 1; }
uv run --quiet --locked --package onetaskgraph-sdk onetaskgraph --help | grep -q 'Usage:' || { echo "Python SDK dependency did not supply the real command; next: inspect the SDK carrier dependency" >&2; exit 1; }
mkdir -p "$tmp/python-wheel"
uv build --quiet --wheel "$root" --out-dir "$tmp/python-wheel"
cli_wheel=$(find "$tmp/python-wheel" -maxdepth 1 -type f -name 'onetaskgraph_cli-*.whl' -print -quit)
[[ -n $cli_wheel ]] || { echo "Python CLI build produced no wheel; next: inspect the maturin package configuration" >&2; exit 1; }
uv run --quiet --isolated --no-project --with "$cli_wheel" onetaskgraph --help | grep -q 'Usage:' || { echo "installed Python CLI wheel did not render help; next: inspect the wheel's binary entry point" >&2; exit 1; }
assert_version_error() {
  expected_message=$1
  shift
  if "$root/scripts/set-version.sh" "$@" 2>"$tmp/error"; then version_error_status=0; else version_error_status=$?; fi
  [[ $version_error_status -eq 2 ]] || { echo "version updater invalid arguments '$*' exited $version_error_status, expected 2; next: inspect argument validation" >&2; exit 1; }
  grep -Fq "$expected_message" "$tmp/error" || { cat "$tmp/error" >&2; echo "version updater invalid arguments '$*' omitted its reason; next: inspect version diagnostics" >&2; exit 1; }
}
assert_version_error 'usage: scripts/set-version.sh VERSION | --check'
assert_version_error 'unexpected extra arguments' 1.2.3 extra
assert_version_error 'invalid semantic version: invalid' invalid
assert_version_error 'invalid semantic version: 1.2.3+build+again' '1.2.3+build+again'
assert_version_error 'unexpected extra arguments after --check' --check ignored
# shellcheck source=scripts/scratch-clone.sh
source "$root/scripts/scratch-clone.sh"
scratch_clone "$root" "$tmp/version-repo"
node -e 'const fs=require("fs"),f=process.argv[1],p=JSON.parse(fs.readFileSync(f));p.version="9.9.9";fs.writeFileSync(f,JSON.stringify(p,null,2)+"\n")' "$tmp/version-repo/npm/cli/package.json"
if "$tmp/version-repo/scripts/set-version.sh" --check 2>"$tmp/error"; then echo "version drift was accepted; next: inspect version checking" >&2; exit 1; fi
grep -q 'version drift found' "$tmp/error" || { cat "$tmp/error" >&2; echo "version-drift failure omitted recovery guidance; next: inspect version diagnostics" >&2; exit 1; }
git -C "$tmp/version-repo" restore npm/cli/package.json
"$tmp/version-repo/scripts/set-version.sh" 0.1.1
grep -q '^version = "0.1.1"' "$tmp/version-repo/Cargo.toml" || { echo "version updater missed the workspace manifest; next: inspect manifest mutation" >&2; exit 1; }
grep -q 'onetaskgraph-cli==0.1.1' "$tmp/version-repo/sdks/python/pyproject.toml" || { echo "version updater missed the Python CLI pin; next: inspect dependency mutation" >&2; exit 1; }
node -e 'const p=require(process.argv[1]); if(p.version!=="0.1.1" || Object.values(p.optionalDependencies).some(v=>v!=="0.1.1")) process.exit(1)' "$tmp/version-repo/npm/cli/package.json" || { echo "version updater missed npm metadata; next: inspect JSON mutation" >&2; exit 1; }
grep -q 'version = "0.1.1"' "$tmp/version-repo/Cargo.lock" || { echo "version updater missed Cargo.lock; next: inspect lock refresh" >&2; exit 1; }
grep -q 'version = "0.1.1"' "$tmp/version-repo/uv.lock" || { echo "version updater missed uv.lock; next: inspect lock refresh" >&2; exit 1; }
