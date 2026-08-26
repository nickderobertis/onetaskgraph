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
command -v npm >/dev/null || { echo "distribution test requires the npm client; next: install Node.js and rerun scripts/test-distribution.sh" >&2; exit 1; }
"$python_bin" -c 'import sys; raise SystemExit(sys.version_info < (3, 8))' || { echo "distribution test requires Python 3.8 or newer; next: install a supported python3 and rerun scripts/test-distribution.sh" >&2; exit 1; }
stop_server() {
  [[ -n ${1:-} ]] || return 0
  kill "$1" 2>/dev/null || true
  wait "$1" 2>/dev/null || true
}
start_server() {
  server_label=$1
  server_program=$2
  shift 2
  server_port_file="$tmp/$server_label.port"
  server_log="$tmp/$server_label.log"
  "$python_bin" "$server_program" "$server_port_file" "$@" >"$server_log" 2>&1 &
  server_pid=$!
  server_exit=
  for _ in {1..600}; do
    [[ -s $server_port_file ]] && break
    if ! kill -0 "$server_pid" 2>/dev/null; then
      if wait "$server_pid"; then server_exit=0; else server_exit=$?; fi
      server_pid=
      break
    fi
    sleep 0.05
  done
  if [[ ! -s $server_port_file ]]; then
    if [[ -n $server_exit ]]; then
      echo "local HTTP $server_label server exited with status $server_exit before starting; server output follows:" >&2
    else
      echo "local HTTP $server_label server was still running after 30 seconds; server output follows:" >&2
    fi
    sed 's/^/  /' "$server_log" >&2
    echo "next: inspect the distribution test server" >&2
    exit 1
  fi
  server_port=$(<"$server_port_file")
  [[ $server_port =~ ^[0-9]+$ && $server_port -ge 1 && $server_port -le 65535 ]] || {
    echo "local HTTP $server_label server reported an unusable port '$server_port'" >&2
    echo "next: inspect the distribution test server" >&2
    exit 1
  }
}
cleanup() {
  stop_server "${http_server_pid:-}"
  stop_server "${registry_server_pid:-}"
  stop_server "${npm_registry_server_pid:-}"
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
cat > "$tmp/release-server.py" <<'PY'
import http.server
import os
import sys

port_file, root = sys.argv[1:]
os.chdir(root)
server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), http.server.SimpleHTTPRequestHandler)
with open(port_file, "w", encoding="utf-8") as stream:
    stream.write(str(server.server_port))
server.serve_forever()
PY
start_server release "$tmp/release-server.py" "$tmp"
http_server_pid=$server_pid
ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="http://127.0.0.1:$server_port/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" >/dev/null
stop_server "$http_server_pid"
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
if (cd "$tmp/version-repo" && python3 scripts/product_versions.py) 2>"$tmp/error"; then helper_usage_status=0; else helper_usage_status=$?; fi
[[ $helper_usage_status -eq 2 ]] || { echo "product-version helper usage failure exited $helper_usage_status, expected 2; next: inspect argument validation" >&2; exit 1; }
grep -q 'usage: scripts/product_versions.py' "$tmp/error" || { cat "$tmp/error" >&2; echo "product-version helper usage failure omitted its reason; next: inspect argument diagnostics" >&2; exit 1; }
if (cd "$tmp/version-repo" && python3 scripts/product_versions.py set 01.2.3) 2>"$tmp/error"; then helper_version_status=0; else helper_version_status=$?; fi
[[ $helper_version_status -eq 2 ]] || { echo "product-version helper invalid version exited $helper_version_status, expected 2; next: inspect version validation" >&2; exit 1; }
grep -q 'invalid semantic version' "$tmp/error" || { cat "$tmp/error" >&2; echo "product-version helper invalid version omitted its reason; next: inspect version diagnostics" >&2; exit 1; }
cp "$tmp/version-repo/Cargo.toml" "$tmp/version-repo/Cargo.toml.valid"
perl -pi -e 'if (/^\[workspace\.package\]/ .. /^version = /) { s/^version = "[^"]+"/version = "01.2.3"/ }' "$tmp/version-repo/Cargo.toml"
if (cd "$tmp/version-repo" && python3 scripts/product_versions.py check 0.1.0) 2>"$tmp/error"; then echo "product-version helper accepted an invalid manifest version; next: inspect manifest validation" >&2; exit 1; fi
grep -q 'Cargo.toml has None; expected 0.1.0' "$tmp/error" || { cat "$tmp/error" >&2; echo "invalid manifest version failure omitted its location; next: inspect version diagnostics" >&2; exit 1; }
# llmlint: ignore[work_goes_through_command_surface] This journey must check the scratch tree directly; the just recipe addresses the outer working tree.
if (cd "$tmp/version-repo" && bash scripts/check-workspace-config.sh) 2>"$tmp/error"; then echo "workspace check accepted an invalid semantic product version; next: inspect workspace version validation" >&2; exit 1; fi
grep -q 'Cargo.toml: no product version could be read' "$tmp/error" || { cat "$tmp/error" >&2; echo "workspace invalid-version failure omitted its location; next: inspect workspace diagnostics" >&2; exit 1; }
mv "$tmp/version-repo/Cargo.toml.valid" "$tmp/version-repo/Cargo.toml"
node -e 'const fs=require("fs"),f=process.argv[1],p=JSON.parse(fs.readFileSync(f));p.version="9.9.9";fs.writeFileSync(f,JSON.stringify(p,null,2)+"\n")' "$tmp/version-repo/npm/cli/package.json"
if "$tmp/version-repo/scripts/set-version.sh" --check 2>"$tmp/error"; then echo "version drift was accepted; next: inspect version checking" >&2; exit 1; fi
grep -q 'version drift found' "$tmp/error" || { cat "$tmp/error" >&2; echo "version-drift failure omitted recovery guidance; next: inspect version diagnostics" >&2; exit 1; }
git -C "$tmp/version-repo" restore npm/cli/package.json
"$tmp/version-repo/scripts/set-version.sh" 0.1.1
# llmlint: ignore[work_goes_through_command_surface] This acceptance journey must reconcile the scratch tree against this exact script; the just recipe addresses the outer working tree.
if ! (cd "$tmp/version-repo" && bash scripts/check-workspace-config.sh) 2>"$tmp/error"; then
  cat "$tmp/error" >&2
  echo "version updater left the workspace version copies inconsistent; next: reconcile its version-file inventory with check-workspace-config.sh" >&2
  exit 1
fi
grep -q '^version = "0.1.1"' "$tmp/version-repo/Cargo.toml" || { echo "version updater missed the workspace manifest; next: inspect manifest mutation" >&2; exit 1; }
grep -q '^__version__ = "0.1.1"' "$tmp/version-repo/sdks/python/src/onetaskgraph_sdk/__init__.py" || { echo "version updater missed the Python SDK module version; next: inspect product-version mutation" >&2; exit 1; }
grep -q 'onetaskgraph-cli==0.1.1' "$tmp/version-repo/sdks/python/pyproject.toml" || { echo "version updater missed the Python CLI pin; next: inspect dependency mutation" >&2; exit 1; }
node -e 'const p=require(process.argv[1]); if(p.version!=="0.1.1" || Object.values(p.optionalDependencies).some(v=>v!=="0.1.1")) process.exit(1)' "$tmp/version-repo/npm/cli/package.json" || { echo "version updater missed npm metadata; next: inspect JSON mutation" >&2; exit 1; }
printf '{\n' > "$tmp/version-repo/sdks/typescript/package.json"
for version_command in '--check' '0.1.2'; do
  if (cd "$tmp/version-repo" && scripts/set-version.sh $version_command) 2>"$tmp/error"; then
    echo "version updater accepted malformed package JSON during '$version_command'; next: inspect manifest validation" >&2
    exit 1
  fi
  grep -q 'product version files could not be processed' "$tmp/error" || { cat "$tmp/error" >&2; echo "malformed product manifest failure omitted recovery guidance; next: inspect version diagnostics" >&2; exit 1; }
done
# llmlint: ignore[work_goes_through_command_surface] This failure journey must run reconciliation inside its deliberately malformed scratch tree.
if (cd "$tmp/version-repo" && bash scripts/check-workspace-config.sh) 2>"$tmp/error"; then echo "workspace check accepted malformed product JSON; next: inspect workspace manifest validation" >&2; exit 1; fi
grep -q 'product version files could not be read' "$tmp/error" || { cat "$tmp/error" >&2; echo "workspace check malformed-manifest failure omitted recovery guidance; next: inspect workspace diagnostics" >&2; exit 1; }
printf '[]\n' > "$tmp/version-repo/sdks/typescript/package.json"
if (cd "$tmp/version-repo" && python3 scripts/product_versions.py check 0.1.1) 2>"$tmp/error"; then echo "product-version helper accepted a non-object package manifest; next: inspect JSON boundary validation" >&2; exit 1; fi
grep -q 'package manifest must be a JSON object' "$tmp/error" || { cat "$tmp/error" >&2; echo "non-object manifest failure omitted its reason; next: inspect JSON diagnostics" >&2; exit 1; }
printf '{}\n' > "$tmp/version-repo/sdks/typescript/package.json"
if (cd "$tmp/version-repo" && python3 scripts/product_versions.py check 0.1.1) 2>"$tmp/error"; then echo "product-version helper accepted a missing JSON version; next: inspect version-field validation" >&2; exit 1; fi
grep -q 'sdks/typescript/package.json has None' "$tmp/error" || { cat "$tmp/error" >&2; echo "missing JSON version failure omitted its location; next: inspect version diagnostics" >&2; exit 1; }
if (cd "$tmp/version-repo" && python3 scripts/product_versions.py set 0.1.2) 2>"$tmp/error"; then echo "product-version helper rewrote a tree with a missing JSON version; next: inspect pre-write validation" >&2; exit 1; fi
grep -q 'no valid semantic product version could be read' "$tmp/error" || { cat "$tmp/error" >&2; echo "set failure for a missing version omitted its reason; next: inspect version diagnostics" >&2; exit 1; }
mv "$tmp/version-repo/sdks/typescript/package.json" "$tmp/version-repo/sdks/typescript/package.json.missing"
if (cd "$tmp/version-repo" && python3 scripts/product_versions.py check 0.1.1) 2>"$tmp/error"; then echo "product-version helper accepted a missing product manifest; next: inspect filesystem error handling" >&2; exit 1; fi
grep -q 'product version files could not be processed' "$tmp/error" || { cat "$tmp/error" >&2; echo "missing-manifest helper failure omitted recovery guidance; next: inspect version diagnostics" >&2; exit 1; }
# llmlint: ignore[work_goes_through_command_surface] The missing-manifest state exists only in the scratch tree, which the outer just recipe cannot address.
if (cd "$tmp/version-repo" && bash scripts/check-workspace-config.sh) 2>"$tmp/error"; then echo "workspace check accepted a missing product manifest; next: inspect filesystem error handling" >&2; exit 1; fi
grep -q 'product version files could not be read' "$tmp/error" || { cat "$tmp/error" >&2; echo "workspace missing-manifest failure omitted recovery guidance; next: inspect workspace diagnostics" >&2; exit 1; }
mv "$tmp/version-repo/sdks/typescript/package.json.missing" "$tmp/version-repo/sdks/typescript/package.json"
git -C "$tmp/version-repo" restore sdks/typescript/package.json
for readonly_manifest in Cargo.toml sdks/typescript/package.json; do
  chmod 444 "$tmp/version-repo/$readonly_manifest"
  if (cd "$tmp/version-repo" && python3 scripts/product_versions.py set 0.1.2) 2>"$tmp/error"; then echo "product-version helper rewrote read-only $readonly_manifest; next: inspect write error handling" >&2; exit 1; fi
  grep -q 'product version files could not be processed' "$tmp/error" || { cat "$tmp/error" >&2; echo "read-only manifest failure omitted recovery guidance; next: inspect version diagnostics" >&2; exit 1; }
  grep -q '^version = "0.1.1"' "$tmp/version-repo/Cargo.toml" || { echo "failed product-version update partially rewrote the workspace manifest; next: inspect write preflight" >&2; exit 1; }
  chmod 644 "$tmp/version-repo/$readonly_manifest"
  git -C "$tmp/version-repo" restore .
  "$tmp/version-repo/scripts/set-version.sh" 0.1.1
done
grep -q 'version = "0.1.1"' "$tmp/version-repo/Cargo.lock" || { echo "version updater missed Cargo.lock; next: inspect lock refresh" >&2; exit 1; }
grep -q 'version = "0.1.1"' "$tmp/version-repo/uv.lock" || { echo "version updater missed uv.lock; next: inspect lock refresh" >&2; exit 1; }
cat > "$tmp/registry-server.py" <<'PY'
import http.server
import sys

port_file, agent_log = sys.argv[1:]
answers = {"published-crate": 200, "absent-crate": 404, "declined-crate": 403, "broken-crate": 500}


class Registry(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        agent = self.headers.get("User-Agent", "")
        with open(agent_log, "a", encoding="utf-8") as stream:
            stream.write(f"{self.path} {agent}\n")
        # crates.io answers curl's default agent with 403, which is the defect under test.
        if not agent or agent.startswith("curl/"):
            self.send_error(403)
            return
        answer = answers.get(self.path.rsplit("/", 2)[-2], 404)
        if answer == 200:
            body = b'{"version":{"num":"1.0.0"}}'
            self.send_response(200)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_error(answer)

    def log_message(self, *args):
        pass


server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Registry)
with open(port_file, "w", encoding="utf-8") as stream:
    stream.write(str(server.server_port))
server.serve_forever()
PY
start_server registry "$tmp/registry-server.py" "$tmp/registry-agents"
registry_server_pid=$server_pid
registry_base="http://127.0.0.1:$server_port/api/v1/crates"
publication_status() { ONETASKGRAPH_CRATES_API_BASE_URL="$registry_base" "$root/scripts/crate-publication-status.sh" "$@"; }
unidentified=$(curl -sS -o /dev/null -w '%{http_code}' "$registry_base/absent-crate/1.0.0")
[[ $unidentified == 403 ]] || { echo "registry stub answered an unidentified caller $unidentified, expected 403; next: inspect the crates.io stub" >&2; exit 1; }
[[ $(publication_status absent-crate 1.0.0) == absent ]] || { echo "an unpublished crate was not reported absent; next: inspect scripts/crate-publication-status.sh" >&2; exit 1; }
[[ $(publication_status published-crate 1.0.0) == published ]] || { echo "a published crate was not reported published; next: inspect scripts/crate-publication-status.sh" >&2; exit 1; }
grep -q '^/api/v1/crates/absent-crate/1.0.0 onetaskgraph-release (https://github.com/nickderobertis/onetaskgraph)$' "$tmp/registry-agents" || { cat "$tmp/registry-agents" >&2; echo "the publication query did not identify itself to the registry; next: inspect the user agent it sends" >&2; exit 1; }
assert_publication_stops() {
  expected_reason=$1
  shift
  if publication_status "$@" >"$tmp/publication" 2>"$tmp/error"; then publication_exit=0; else publication_exit=$?; fi
  [[ $publication_exit -eq 69 ]] || { cat "$tmp/error" >&2; echo "unusable crates.io answer for '$*' exited $publication_exit, expected 69; next: inspect scripts/crate-publication-status.sh" >&2; exit 1; }
  [[ ! -s $tmp/publication ]] || { cat "$tmp/publication" >&2; echo "unusable crates.io answer for '$*' still reported a publication decision; next: inspect scripts/crate-publication-status.sh" >&2; exit 1; }
  grep -q "$expected_reason" "$tmp/error" || { cat "$tmp/error" >&2; echo "unusable crates.io answer for '$*' omitted its reason; next: inspect publication diagnostics" >&2; exit 1; }
  grep -q '^next: ' "$tmp/error" || { cat "$tmp/error" >&2; echo "unusable crates.io answer for '$*' omitted a next action; next: inspect publication diagnostics" >&2; exit 1; }
}
assert_publication_stops 'declined the caller' declined-crate 1.0.0
assert_publication_stops 'does not say whether it is published' broken-crate 1.0.0
stop_server "$registry_server_pid"
registry_server_pid=
assert_publication_stops 'could not reach crates.io' absent-crate 1.0.0
assert_publication_refuses() {
  expected_reason=$1
  shift
  if "$@" >"$tmp/publication" 2>"$tmp/error"; then publication_usage_status=0; else publication_usage_status=$?; fi
  [[ $publication_usage_status -eq 64 ]] || { cat "$tmp/error" >&2; echo "publication status for '$expected_reason' exited $publication_usage_status, expected 64; next: inspect argument validation" >&2; exit 1; }
  [[ ! -s $tmp/publication ]] || { cat "$tmp/publication" >&2; echo "a refused publication query still reported a decision; next: inspect scripts/crate-publication-status.sh" >&2; exit 1; }
  grep -Fq 'usage: scripts/crate-publication-status.sh CRATE VERSION' "$tmp/error" || { cat "$tmp/error" >&2; echo "publication status refusal omitted its usage line; next: inspect publication diagnostics" >&2; exit 1; }
  grep -Fq "next: $expected_reason" "$tmp/error" || { cat "$tmp/error" >&2; echo "publication status refusal omitted its next action; next: inspect publication diagnostics" >&2; exit 1; }
}
assert_publication_refuses 'name one crate and its X.Y.Z version' "$root/scripts/crate-publication-status.sh" onetaskgraph
assert_publication_refuses 'invalid crate name: ../onetaskgraph' "$root/scripts/crate-publication-status.sh" ../onetaskgraph 1.0.0
assert_publication_refuses 'invalid version for onetaskgraph: 1.0' "$root/scripts/crate-publication-status.sh" onetaskgraph 1.0
assert_publication_refuses 'invalid registry base, which must be an http:// or https:// URL: file:///etc' env ONETASKGRAPH_CRATES_API_BASE_URL=file:///etc "$root/scripts/crate-publication-status.sh" onetaskgraph 1.0.0
cat > "$tmp/npm-registry-server.py" <<'PY'
import http.server
import sys

port_file, expected_token, request_log = sys.argv[1:]


class NpmRegistry(http.server.BaseHTTPRequestHandler):
    def record(self, verb):
        authorization = self.headers.get("Authorization", "")
        # Records how the caller authenticated, never the credential it sent.
        if authorization == f"Bearer {expected_token}":
            state = "authorized"
        elif authorization:
            state = "rejected"
        else:
            state = "anonymous"
        with open(request_log, "a", encoding="utf-8") as stream:
            stream.write(f"{verb} {self.path} {state}\n")
        return state

    def do_GET(self):
        self.record("GET")
        self.send_error(404)

    def do_PUT(self):
        declared = self.headers.get("Content-Length", "0")
        # A carrier tarball is a few megabytes. Anything unreadable as a length, or larger
        # than this stub will hold, is refused rather than read.
        if not declared.isdigit() or int(declared) > 64 * 1024 * 1024:
            self.send_error(400, "unusable Content-Length")
            return
        self.rfile.read(int(declared))
        if self.record("PUT") != "authorized":
            self.send_error(401)
            return
        body = b'{"ok":true}'
        self.send_response(201)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), NpmRegistry)
with open(port_file, "w", encoding="utf-8") as stream:
    stream.write(str(server.server_port))
server.serve_forever()
PY
npm_token=distribution-test-token-not-a-credential
start_server npm-registry "$tmp/npm-registry-server.py" "$npm_token" "$tmp/npm-requests"
npm_registry_server_pid=$server_pid
# The publication below is driven at the registry URL without its trailing slash, so the
# normalization npm's auth key needs is proven by the real client rather than asserted.
npm_registry="http://127.0.0.1:$server_port"
npm_registry_url="$npm_registry/"
probe=@onetaskgraph/cli-publication-probe
mkdir -p "$tmp/npm-package" "$tmp/npm-config" "$tmp/npm-config-without-auth"
printf 'module.exports = {};\n' > "$tmp/npm-package/index.js"
node -e 'require("fs").writeFileSync(process.argv[1], JSON.stringify({name: process.argv[2], version: process.argv[3], main: "index.js"}, null, 2) + "\n")' "$tmp/npm-package/package.json" "$probe" "$version"
npm_config=$(ONETASKGRAPH_NPM_CONFIG_DIR="$tmp/npm-config" "$root/scripts/npm-registry-auth.sh" "$npm_registry")
[[ -s $tmp/npm-config/.npmrc ]] || { echo "the npm registry configuration was not written; next: inspect scripts/npm-registry-auth.sh" >&2; exit 1; }
grep -Fq "registry=$npm_registry_url" "$tmp/npm-config/.npmrc" || { cat "$tmp/npm-config/.npmrc" >&2; echo "the npm configuration did not point the client at the registry it was given; next: inspect scripts/npm-registry-auth.sh" >&2; exit 1; }
grep -Fq ':_authToken=${NODE_AUTH_TOKEN}' "$tmp/npm-config/.npmrc" || { cat "$tmp/npm-config/.npmrc" >&2; echo "the npm configuration did not name NODE_AUTH_TOKEN as the registry's auth token; next: inspect scripts/npm-registry-auth.sh" >&2; exit 1; }
if grep -Fq "$npm_token" "$tmp/npm-config/.npmrc"; then echo "the npm configuration recorded a token value instead of naming the variable; next: inspect scripts/npm-registry-auth.sh" >&2; exit 1; fi
# The defect this journey pins: a job that exports NODE_AUTH_TOKEN and configures the
# registry but no auth token packs in full and then fails as though it were logged out.
unauthenticated_config=$(ONETASKGRAPH_NPM_CONFIG_DIR="$tmp/npm-config-without-auth" "$root/scripts/npm-registry-auth.sh" "$npm_registry")
grep -Fv ':_authToken=' "$tmp/npm-config-without-auth/.npmrc" > "$tmp/npmrc-without-auth"
cp "$tmp/npmrc-without-auth" "$tmp/npm-config-without-auth/.npmrc"
if NPM_CONFIG_USERCONFIG="$unauthenticated_config" NODE_AUTH_TOKEN="$npm_token" npm publish "$tmp/npm-package" --access public --cache "$tmp/npm-cache" --no-update-notifier >"$tmp/npm-output" 2>&1; then
  cat "$tmp/npm-output" >&2
  echo "npm published with no registry authentication configured; next: inspect the local npm registry stub" >&2
  exit 1
fi
grep -q 'ENEEDAUTH' "$tmp/npm-output" || { cat "$tmp/npm-output" >&2; echo "an unconfigured npm publish failed for some reason other than missing authentication; next: inspect the local npm registry stub" >&2; exit 1; }
if [[ -s $tmp/npm-requests ]] && grep -q 'cli-publication-probe' "$tmp/npm-requests"; then cat "$tmp/npm-requests" >&2; echo "an unauthenticated npm publish still reached the registry; next: inspect the local npm registry stub" >&2; exit 1; fi
NPM_CONFIG_USERCONFIG="$npm_config" NODE_AUTH_TOKEN="$npm_token" npm publish "$tmp/npm-package" --access public --cache "$tmp/npm-cache" --no-update-notifier >"$tmp/npm-output" 2>&1 || { cat "$tmp/npm-output" >&2; echo "npm could not publish with the configuration scripts/npm-registry-auth.sh writes; next: inspect that configuration" >&2; exit 1; }
grep -Fq "+ $probe@$version" "$tmp/npm-output" || { cat "$tmp/npm-output" >&2; echo "npm publish did not report the published package; next: inspect the local npm registry stub" >&2; exit 1; }
grep -q '^PUT /@onetaskgraph%2fcli-publication-probe authorized$' "$tmp/npm-requests" || { cat "$tmp/npm-requests" >&2; echo "the npm publication did not authenticate with the token it was given; next: inspect scripts/npm-registry-auth.sh" >&2; exit 1; }
stop_server "$npm_registry_server_pid"
npm_registry_server_pid=
assert_npm_auth_refuses() {
  expected_reason=$1
  shift
  if "$root/scripts/npm-registry-auth.sh" "$@" >"$tmp/npm-config-path" 2>"$tmp/error"; then npm_auth_status=0; else npm_auth_status=$?; fi
  [[ $npm_auth_status -eq 64 ]] || { cat "$tmp/error" >&2; echo "npm registry configuration for '$*' exited $npm_auth_status, expected 64; next: inspect argument validation" >&2; exit 1; }
  [[ ! -s $tmp/npm-config-path ]] || { cat "$tmp/npm-config-path" >&2; echo "a refused npm registry configuration still reported a path; next: inspect scripts/npm-registry-auth.sh" >&2; exit 1; }
  grep -Fq 'usage: scripts/npm-registry-auth.sh [REGISTRY_URL]' "$tmp/error" || { cat "$tmp/error" >&2; echo "npm registry configuration refusal omitted its usage line; next: inspect its diagnostics" >&2; exit 1; }
  grep -Fq "next: $expected_reason" "$tmp/error" || { cat "$tmp/error" >&2; echo "npm registry configuration refusal omitted its next action; next: inspect its diagnostics" >&2; exit 1; }
}
assert_npm_auth_refuses 'invalid registry, which must be an http:// or https:// URL naming a host, with an optional port: file:///etc' file:///etc
assert_npm_auth_refuses 'pass at most one registry URL' https://registry.npmjs.org/ extra
assert_npm_auth_refuses 'invalid registry, which must be an http:// or https:// URL naming a host, with an optional port: http:///' http:///
# An authority is not whatever follows the scheme: a query or a fragment there leaves no
# host at all, and npm would key the token to that punctuation and publish anonymously.
assert_npm_auth_refuses 'invalid registry, which must be an http:// or https:// URL naming a host, with an optional port: https://?registry=x' 'https://?registry=x'
assert_npm_auth_refuses 'invalid registry, which must be an http:// or https:// URL naming a host, with an optional port: https://#registry' 'https://#registry'
assert_npm_auth_refuses 'invalid registry, which must be an http:// or https:// URL naming a host, with an optional port: https://token@' 'https://token@'
# A bracketed address literal is refused rather than half-checked, so the refusal is
# asserted here: what would otherwise pass is a grammar this validation cannot judge.
assert_npm_auth_refuses 'invalid registry, which must be an http:// or https:// URL naming a host, with an optional port: http://[::1]:8080' 'http://[::1]:8080'
# A port is judged as a number, which no pattern above it can do, and every port refused
# here would otherwise have keyed the token to a port nothing can listen on.
assert_npm_auth_refuses 'invalid registry port, which must be between 1 and 65535: http://127.0.0.1:0' 'http://127.0.0.1:0'
assert_npm_auth_refuses 'invalid registry port, which must be between 1 and 65535: http://127.0.0.1:70000' 'http://127.0.0.1:70000'
assert_npm_auth_refuses 'invalid registry port, which must be between 1 and 65535: http://127.0.0.1:99999999999999999999' 'http://127.0.0.1:99999999999999999999'
# The bound is inclusive at the top, so the highest usable port is still a registry.
ONETASKGRAPH_NPM_CONFIG_DIR="$tmp/npm-config-high-port" "$root/scripts/npm-registry-auth.sh" 'http://127.0.0.1:65535' >/dev/null
grep -Fq '//127.0.0.1:65535/:_authToken=${NODE_AUTH_TOKEN}' "$tmp/npm-config-high-port/.npmrc" || { cat "$tmp/npm-config-high-port/.npmrc" >&2; echo "the npm configuration did not key the token to the highest usable port; next: inspect scripts/npm-registry-auth.sh" >&2; exit 1; }
if ONETASKGRAPH_NPM_CONFIG_DIR="$tmp/npm-config-unreportable" "$root/scripts/npm-registry-auth.sh" "$npm_registry" >&- 2>"$tmp/error"; then echo "the npm configuration reported a path over a standard output it could not write; next: inspect scripts/npm-registry-auth.sh" >&2; exit 1; fi
grep -Fq 'could not report the npm configuration path' "$tmp/error" || { cat "$tmp/error" >&2; echo "an unreportable npm configuration path omitted its reason; next: inspect its diagnostics" >&2; exit 1; }
grep -q '^next: ' "$tmp/error" || { cat "$tmp/error" >&2; echo "an unreportable npm configuration path omitted a next action; next: inspect its diagnostics" >&2; exit 1; }
assert_npm_auth_stops() {
  expected_reason=$1
  shift
  if env "$@" "$root/scripts/npm-registry-auth.sh" "$npm_registry" >"$tmp/npm-config-path" 2>"$tmp/error"; then npm_auth_status=0; else npm_auth_status=$?; fi
  [[ $npm_auth_status -eq 1 ]] || { cat "$tmp/error" >&2; echo "npm registry configuration for '$*' exited $npm_auth_status, expected 1; next: inspect scripts/npm-registry-auth.sh" >&2; exit 1; }
  [[ ! -s $tmp/npm-config-path ]] || { cat "$tmp/npm-config-path" >&2; echo "an unwritable npm registry configuration still reported a path; next: inspect scripts/npm-registry-auth.sh" >&2; exit 1; }
  grep -Fq "$expected_reason" "$tmp/error" || { cat "$tmp/error" >&2; echo "an unwritable npm registry configuration omitted its reason; next: inspect its diagnostics" >&2; exit 1; }
  grep -q '^next: ' "$tmp/error" || { cat "$tmp/error" >&2; echo "an unwritable npm registry configuration omitted a next action; next: inspect its diagnostics" >&2; exit 1; }
}
printf 'not a directory' > "$tmp/npm-not-a-directory"
assert_npm_auth_stops 'could not create the npm configuration directory' "ONETASKGRAPH_NPM_CONFIG_DIR=$tmp/npm-not-a-directory/child"
mkdir -p "$tmp/npm-config-unwritable/.npmrc"
assert_npm_auth_stops 'could not write the npm configuration' "ONETASKGRAPH_NPM_CONFIG_DIR=$tmp/npm-config-unwritable"
# The release workflow passes no directory: it takes the runner's temporary tree, and falls
# back to one of its own where there is none.
RUNNER_TEMP="$tmp/runner-temp" "$root/scripts/npm-registry-auth.sh" "$npm_registry" >/dev/null
grep -Fq ':_authToken=${NODE_AUTH_TOKEN}' "$tmp/runner-temp/.npmrc" || { echo "the npm configuration did not land in the runner's temporary tree; next: inspect scripts/npm-registry-auth.sh" >&2; exit 1; }
mkdir -p "$tmp/fallback-temp"
# Which directory mktemp picks is the platform's own business — GNU mktemp honours TMPDIR
# and BSD mktemp ignores it for /tmp — so this reads the path the helper printed, the way
# the release workflow reads it, rather than searching where this side guessed it landed.
fallback_config=$(env -u RUNNER_TEMP TMPDIR="$tmp/fallback-temp" "$root/scripts/npm-registry-auth.sh" "$npm_registry")
[[ -s $fallback_config ]] || { echo "the npm configuration had nowhere to go with no runner temporary tree; next: inspect scripts/npm-registry-auth.sh" >&2; exit 1; }
grep -Fq ':_authToken=${NODE_AUTH_TOKEN}' "$fallback_config" || { cat "$fallback_config" >&2; echo "the fallback npm configuration did not name NODE_AUTH_TOKEN; next: inspect scripts/npm-registry-auth.sh" >&2; exit 1; }
# That directory is the helper's own and can sit outside this journey's tree, so it is
# taken away here rather than left for the cleanup that only reaches $tmp.
rm -f "$fallback_config"
rmdir "$(dirname "$fallback_config")" 2>/dev/null || true
# The two tools this helper leans on are forced to fail the way the installer's own cases
# force theirs, because neither refusal can be provoked on the platform that runs here.
mkdir -p "$tmp/npm-shims"
printf '#!/bin/sh\nexit 1\n' > "$tmp/npm-shims/mktemp"
printf '#!/bin/sh\nexit 1\n' > "$tmp/npm-shims/cygpath"
chmod +x "$tmp/npm-shims/mktemp" "$tmp/npm-shims/cygpath"
assert_npm_auth_stops 'could not create a directory for the npm configuration' -u RUNNER_TEMP "PATH=$tmp/npm-shims:$PATH"
assert_npm_auth_stops 'could not express the npm configuration path for this platform' "ONETASKGRAPH_NPM_CONFIG_DIR=$tmp/npm-config-cygpath" "PATH=$tmp/npm-shims:$PATH"
# The release workflow names no registry either, so the default is what actually publishes.
ONETASKGRAPH_NPM_CONFIG_DIR="$tmp/npm-config-default" "$root/scripts/npm-registry-auth.sh" >/dev/null
grep -Fq 'registry=https://registry.npmjs.org/' "$tmp/npm-config-default/.npmrc" || { cat "$tmp/npm-config-default/.npmrc" >&2; echo "the default npm configuration did not point at the public registry; next: inspect scripts/npm-registry-auth.sh" >&2; exit 1; }
grep -Fq '//registry.npmjs.org/:_authToken=${NODE_AUTH_TOKEN}' "$tmp/npm-config-default/.npmrc" || { cat "$tmp/npm-config-default/.npmrc" >&2; echo "the default npm configuration did not name NODE_AUTH_TOKEN for the public registry; next: inspect scripts/npm-registry-auth.sh" >&2; exit 1; }
scratch_clone "$root" "$tmp/contract-repo"
# The clone supplies the surrounding tree; the three files under test come from the working
# copy, because what the gate has to refuse is the release path as it stands right now.
restore_contract_repo() {
  cp "$root/.github/workflows/release.yml" "$tmp/contract-repo/.github/workflows/release.yml"
  cp "$root/scripts/npm-registry-auth.sh" "$tmp/contract-repo/scripts/npm-registry-auth.sh"
  cp "$root/scripts/check-distribution-contract.sh" "$tmp/contract-repo/scripts/check-distribution-contract.sh"
}
restore_contract_repo
(cd "$tmp/contract-repo" && ./scripts/check-distribution-contract.sh) || { echo "the distribution contract refused the release path it is meant to accept; next: rerun scripts/check-distribution-contract.sh" >&2; exit 1; }
# The workflow read is judged rather than trusted: under `set -e` a sed that cannot open it
# would end the contract on sed's own diagnostic, before the refusal that carries the next
# action. A sed that fails for that one read and no other is what forces the handler.
mkdir -p "$tmp/contract-shims"
printf '#!/bin/sh\ncase "$*" in *publish-npm*) echo "sed: simulated read failure" >&2; exit 2;; esac\nexec %s "$@"\n' "$(command -v sed)" > "$tmp/contract-shims/sed"
chmod +x "$tmp/contract-shims/sed"
if (cd "$tmp/contract-repo" && PATH="$tmp/contract-shims:$PATH" ./scripts/check-distribution-contract.sh) >"$tmp/contract-output" 2>"$tmp/error"; then
  echo "the distribution contract accepted a release workflow it could not read; next: inspect scripts/check-distribution-contract.sh" >&2
  exit 1
fi
grep -Fq 'could not read .github/workflows/release.yml' "$tmp/error" || { cat "$tmp/error" >&2; echo "the distribution contract did not name the file it could not read; next: inspect its diagnostics" >&2; exit 1; }
grep -q '^next: ' "$tmp/error" || { cat "$tmp/error" >&2; echo "the distribution contract read failure omitted a next action; next: inspect its diagnostics" >&2; exit 1; }
assert_contract_refuses() {
  expected_reason=$1
  relative=$2
  removed=$3
  restore_contract_repo
  grep -Fv "$removed" "$tmp/contract-repo/$relative" > "$tmp/contract-mutation"
  cp "$tmp/contract-mutation" "$tmp/contract-repo/$relative"
  if (cd "$tmp/contract-repo" && ./scripts/check-distribution-contract.sh) >"$tmp/contract-output" 2>"$tmp/error"; then
    echo "the distribution contract accepted a release with '$removed' removed from $relative; next: inspect scripts/check-distribution-contract.sh" >&2
    exit 1
  fi
  grep -Fq "$expected_reason" "$tmp/error" || { cat "$tmp/error" >&2; echo "the distribution contract refused '$relative' without naming npm authentication; next: inspect its diagnostics" >&2; exit 1; }
  grep -Fq 'next: restore the npm registry authentication' "$tmp/error" || { cat "$tmp/error" >&2; echo "the distribution contract refused '$relative' without a next action that repairs npm authentication; next: inspect its diagnostics" >&2; exit 1; }
  restore_contract_repo
}
assert_contract_refuses 'NODE_AUTH_TOKEN alone leaves the npm client logged out' .github/workflows/release.yml 'NPM_CONFIG_USERCONFIG=$(scripts/npm-registry-auth.sh'
assert_contract_refuses 'must be exported as NPM_CONFIG_USERCONFIG' .github/workflows/release.yml 'export NPM_CONFIG_USERCONFIG'
assert_contract_refuses 'must name NODE_AUTH_TOKEN rather than carry a token value' scripts/npm-registry-auth.sh ':_authToken=${NODE_AUTH_TOKEN}'
assert_contract_refuses 'no publish-npm job to authenticate' .github/workflows/release.yml '  publish-npm:'
