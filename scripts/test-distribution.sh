#!/usr/bin/env bash
set -euo pipefail
report_failure() {
  echo "distribution setup failed at line $2 (exit $1): $3; next: rerun scripts/test-distribution.sh and fix that command" >&2
}
trap 'report_failure "$?" "$LINENO" "$BASH_COMMAND"' ERR
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
install_output=$(ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh")
printf '%s\n' "$install_output" | grep -q "installed $tag to" || { echo "installer success omitted its destination; next: inspect the success contract" >&2; exit 1; }
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
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/missing" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "missing checksum accepted; next: inspect checksum download handling" >&2; exit 1; fi
grep -q 'checksum download failed' "$tmp/error" || { echo "checksum download failure omitted its reason; next: inspect download diagnostics" >&2; exit 1; }
printf 'not-a-digest\n' > "$tmp/canonical/$tag/$name.sha256"
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "malformed checksum accepted; next: inspect checksum parsing" >&2; exit 1; fi
grep -q 'does not contain one SHA-256 digest' "$tmp/error" || { echo "malformed checksum failure omitted its reason; next: inspect checksum diagnostics" >&2; exit 1; }
printf '%064d first\n%064d second\n' 0 1 > "$tmp/canonical/$tag/$name.sha256"
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "multiple checksum records accepted; next: inspect checksum cardinality" >&2; exit 1; fi
grep -q 'exactly one SHA-256 record' "$tmp/error" || { echo "multi-record checksum failure omitted its reason; next: inspect checksum diagnostics" >&2; exit 1; }
python - "$tmp/releases/$tag/$name" "$ext" <<'PY'
import io
import sys
import tarfile
import zipfile

path, extension = sys.argv[1:]
if extension == "zip":
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr("../escape", b"unsafe")
else:
    with tarfile.open(path, "w:gz") as archive:
        member = tarfile.TarInfo("../escape")
        member.size = 6
        archive.addfile(member, io.BytesIO(b"unsafe"))
PY
if command -v sha256sum >/dev/null; then sha256sum "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"; else shasum -a 256 "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"; fi
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "unsafe archive member installed; next: inspect member validation" >&2; exit 1; fi
grep -q 'unsafe member path' "$tmp/error" || { echo "unsafe-member failure omitted its reason; next: inspect archive validation" >&2; exit 1; }
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="ftp://invalid/releases" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "unsupported download scheme was accepted; next: inspect source validation" >&2; exit 1; fi
grep -q 'download base must use' "$tmp/error" || { echo "unsupported-scheme failure omitted its reason; next: inspect source diagnostics" >&2; exit 1; }
if [[ $ext == zip ]]; then (cd "$root/target/debug" && 7z a "$tmp/releases/$tag/$name" "$binary" >/dev/null); else tar -czf "$tmp/releases/$tag/$name" -C "$root/target/debug" "$binary"; fi
if command -v sha256sum >/dev/null; then sha256sum "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"; else shasum -a 256 "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"; fi
mkdir -p "$tmp/shims"
printf '#!/bin/sh\necho '\''{"tag_name":"%s"}'\''\n' "$tag" > "$tmp/shims/curl"
chmod +x "$tmp/shims/curl"
PATH="$tmp/shims:$PATH" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" >/dev/null
printf '#!/bin/sh\nexit 1\n' > "$tmp/shims/curl"
if PATH="$tmp/shims:$PATH" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "latest-release lookup failure was accepted; next: inspect release resolution" >&2; exit 1; fi
grep -q 'could not resolve the latest GitHub Release' "$tmp/error" || { echo "latest-release failure omitted its reason; next: inspect release diagnostics" >&2; exit 1; }
printf '#!/bin/sh\nprintf '\''Unsupported\\n'\''\n' > "$tmp/shims/uname"
chmod +x "$tmp/shims/uname"
if PATH="$tmp/shims:$PATH" ONETASKGRAPH_VERSION="$tag" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "unsupported installer platform was accepted; next: inspect platform selection" >&2; exit 1; fi
grep -q 'no prebuilt binary' "$tmp/error" || { echo "unsupported-installer-platform failure omitted its reason; next: inspect platform diagnostics" >&2; exit 1; }
rm "$tmp/shims/uname" "$tmp/shims/curl"
printf 'not a directory' > "$tmp/not-a-directory"
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/not-a-directory/child" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "uncreatable installation directory was accepted; next: inspect directory creation" >&2; exit 1; fi
grep -q 'could not create the installation directory' "$tmp/error" || { echo "directory-creation failure omitted its reason; next: inspect filesystem diagnostics" >&2; exit 1; }
python - "$tmp/releases/$tag/$name" "$ext" <<'PY'
import sys
import tarfile
import zipfile

path, extension = sys.argv[1:]
if extension == "zip":
    with zipfile.ZipFile(path, "w") as archive:
        member = zipfile.ZipInfo("linked")
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
grep -q 'archive contains a link entry' "$tmp/error" || { echo "link-entry failure omitted its reason; next: inspect archive diagnostics" >&2; exit 1; }
if [[ $ext == zip ]]; then (cd "$tmp" && 7z a "$tmp/releases/$tag/$name" error >/dev/null); else tar -czf "$tmp/releases/$tag/$name" -C "$tmp" error; fi
if command -v sha256sum >/dev/null; then sha256sum "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"; else shasum -a 256 "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"; fi
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "archive without binary was accepted; next: inspect binary validation" >&2; exit 1; fi
grep -q 'archive binary is not a regular file' "$tmp/error" || { echo "missing-archive-binary failure omitted its reason; next: inspect archive diagnostics" >&2; exit 1; }
if [[ $ext == zip ]]; then (cd "$root/target/debug" && 7z a "$tmp/releases/$tag/$name" "$binary" >/dev/null); else tar -czf "$tmp/releases/$tag/$name" -C "$root/target/debug" "$binary"; fi
if command -v sha256sum >/dev/null; then sha256sum "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"; else shasum -a 256 "$tmp/releases/$tag/$name" > "$tmp/canonical/$tag/$name.sha256"; fi
mkdir "$tmp/no-hash-path"
for utility in awk cp dirname grep install mkdir mktemp rm sed tar uname; do ln -s "$(command -v "$utility")" "$tmp/no-hash-path/$utility"; done
if PATH="$tmp/no-hash-path" ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "installer accepted a system without SHA-256 tooling; next: inspect hash-tool detection" >&2; exit 1; fi
grep -q 'no SHA-256 implementation is installed' "$tmp/error" || { echo "missing-hash-tool failure omitted its reason; next: inspect integrity diagnostics" >&2; exit 1; }
printf '#!/bin/sh\nexit 1\n' > "$tmp/shims/mktemp"
chmod +x "$tmp/shims/mktemp"
if PATH="$tmp/shims:$PATH" ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "temporary-directory failure was accepted; next: inspect temporary setup" >&2; exit 1; fi
grep -q 'could not create a temporary directory' "$tmp/error" || { echo "temporary-directory failure omitted its reason; next: inspect filesystem diagnostics" >&2; exit 1; }
rm "$tmp/shims/mktemp"
if [[ $ext == tar.gz ]]; then
  real_tar=$(command -v tar)
  printf '#!/bin/sh\nif [ "$1" = -tvzf ]; then exit 1; fi\nexec %s "$@"\n' "$real_tar" > "$tmp/shims/tar"
  chmod +x "$tmp/shims/tar"
  if PATH="$tmp/shims:$PATH" ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "unreadable archive metadata was accepted; next: inspect metadata validation" >&2; exit 1; fi
  grep -q 'archive metadata is unreadable' "$tmp/error" || { echo "metadata failure omitted its reason; next: inspect archive diagnostics" >&2; exit 1; }
  printf '#!/bin/sh\nif [ "$1" = -xzf ]; then exit 1; fi\nexec %s "$@"\n' "$real_tar" > "$tmp/shims/tar"
  if PATH="$tmp/shims:$PATH" ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "archive extraction failure was accepted; next: inspect extraction handling" >&2; exit 1; fi
  grep -q 'could not extract' "$tmp/error" || { echo "extraction failure omitted its reason; next: inspect archive diagnostics" >&2; exit 1; }
  rm "$tmp/shims/tar"
fi
printf '#!/bin/sh\nexit 1\n' > "$tmp/shims/install"
chmod +x "$tmp/shims/install"
if PATH="$tmp/shims:$PATH" ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL="file://$tmp/releases" ONETASKGRAPH_CHECKSUM_BASE_URL="file://$tmp/canonical" ONETASKGRAPH_INSTALL_DIR="$tmp/bin" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "final installation failure was accepted; next: inspect executable installation" >&2; exit 1; fi
grep -q 'could not install into' "$tmp/error" || { echo "installation-copy failure omitted its reason; next: inspect filesystem diagnostics" >&2; exit 1; }
rm "$tmp/shims/install"
if ONETASKGRAPH_VERSION="$tag" ONETASKGRAPH_RELEASE_BASE_URL=https://mirror.example/releases ONETASKGRAPH_CHECKSUM_BASE_URL=https://mirror.example/checks "$root/scripts/install.sh" 2>"$tmp/error"; then echo "mirror-controlled checksum was accepted; next: inspect origin comparison" >&2; exit 1; fi
grep -q "checksum shares the mirror's origin" "$tmp/error" || { echo "mirror rejection omitted its reason; next: inspect installer diagnostics" >&2; exit 1; }
if "$root/scripts/install.sh" --version 2>"$tmp/error"; then echo "missing option value was accepted; next: inspect installer argument parsing" >&2; exit 1; fi
grep -q 'requires a value' "$tmp/error" || { echo "missing-value failure omitted its reason; next: inspect argument diagnostics" >&2; exit 1; }
if "$root/scripts/install.sh" --to '' 2>"$tmp/error"; then empty_destination_status=0; else empty_destination_status=$?; fi
[[ $empty_destination_status -eq 64 ]] || { echo "empty installation destination exited $empty_destination_status, expected 64; next: inspect destination validation" >&2; exit 1; }
grep -q 'installation directory must not be empty' "$tmp/error" || { echo "empty-destination failure omitted its reason; next: inspect destination diagnostics" >&2; exit 1; }
if ONETASKGRAPH_VERSION="${tag}junk" "$root/scripts/install.sh" 2>"$tmp/error"; then echo "malformed tag was accepted; next: inspect tag validation" >&2; exit 1; fi
node_platform=$(node -p '`${process.platform}-${process.arch}`')
mkdir -p "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin"
cp "$root/target/debug/$binary" "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/$binary"
printf '{"name":"@onetaskgraph/cli-%s"}\n' "$node_platform" > "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/package.json"
(cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js" --help) | grep -q 'Usage:' || { echo "launcher did not execute the carrier; next: inspect package resolution" >&2; exit 1; }
printf 'not json\n' > "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/package.json"
if (cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js") 2>"$tmp/error"; then echo "launcher accepted malformed carrier JSON; next: inspect manifest validation" >&2; exit 1; fi
grep -q 'invalid @onetaskgraph/cli-' "$tmp/error" || { echo "malformed-carrier failure omitted its reason; next: inspect launcher diagnostics" >&2; exit 1; }
printf '{"name":"wrong-carrier"}\n' > "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/package.json"
if (cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js") 2>"$tmp/error"; then echo "launcher accepted wrong carrier identity; next: inspect manifest validation" >&2; exit 1; fi
grep -q 'identifies itself as wrong-carrier' "$tmp/error" || { echo "wrong-carrier failure omitted its reason; next: inspect launcher diagnostics" >&2; exit 1; }
printf '{"name":"@onetaskgraph/cli-%s"}\n' "$node_platform" > "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/package.json"
mv "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/$binary" "$tmp/real-carrier"
ln -s "$tmp/real-carrier" "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/$binary"
if (cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js") 2>"$tmp/error"; then echo "launcher accepted escaping carrier binary; next: inspect carrier containment" >&2; exit 1; fi
grep -q 'carrier binary escapes its package' "$tmp/error" || { echo "escaping-carrier failure omitted its reason; next: inspect launcher diagnostics" >&2; exit 1; }
rm "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/$binary"
printf '#!/bin/sh\nkill -TERM $$\n' > "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/$binary"
chmod +x "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/$binary"
if (cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js") 2>"$tmp/error"; then signal_status=0; else signal_status=$?; fi
[[ $signal_status -eq 70 ]] || { echo "signaled carrier exited $signal_status, expected 70; next: inspect signal handling" >&2; exit 1; }
grep -q 'carrier terminated by' "$tmp/error" || { echo "signal failure omitted its reason; next: inspect launcher diagnostics" >&2; exit 1; }
mv "$tmp/real-carrier" "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/$binary"
set +e
(cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js" --definitely-invalid >/dev/null 2>&1)
launcher_status=$?
set -e
[[ $launcher_status -eq 2 ]] || { echo "launcher did not propagate command status 2; next: inspect spawn result handling" >&2; exit 1; }
chmod -x "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/$binary"
if (cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js") 2>"$tmp/error"; then echo "launcher executed a non-executable carrier; next: inspect spawn errors" >&2; exit 1; fi
grep -q 'reinstall the platform package' "$tmp/error" || { echo "spawn failure omitted recovery guidance; next: inspect launcher diagnostics" >&2; exit 1; }
chmod +x "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/$binary"
mv "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/$binary" "$tmp/missing-binary"
set +e
(cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js") 2>"$tmp/error"
missing_binary_status=$?
set -e
[[ $missing_binary_status -eq 69 ]] || { echo "missing carrier binary exited $missing_binary_status, expected 69; next: inspect spawn errors" >&2; exit 1; }
grep -q 'reinstall the platform package' "$tmp/error" || { echo "missing-binary failure omitted recovery guidance; next: inspect spawn errors" >&2; exit 1; }
mv "$tmp/missing-binary" "$tmp/node_modules/@onetaskgraph/cli-${node_platform}/bin/$binary"
mv "$tmp/node_modules/@onetaskgraph/cli-${node_platform}" "$tmp/missing-carrier"
if (cd "$tmp" && NODE_PATH="$tmp/node_modules" node "$root/npm/cli/bin/onetaskgraph.js") 2>"$tmp/error"; then echo "launcher accepted a missing carrier; next: inspect package resolution" >&2; exit 1; fi
grep -q 'is not installed' "$tmp/error" || { echo "missing-carrier failure omitted recovery guidance; next: inspect launcher diagnostics" >&2; exit 1; }
set +e
node -e 'Object.defineProperty(process,"platform",{value:"unsupported"}); require(process.argv[1])' "$root/npm/cli/bin/onetaskgraph.js" 2>"$tmp/error"
unsupported_status=$?
set -e
[[ $unsupported_status -eq 64 ]] || { echo "unsupported launcher platform exited $unsupported_status, expected 64; next: inspect platform validation" >&2; exit 1; }
grep -q 'unsupported platform' "$tmp/error" || { echo "unsupported-platform failure omitted its reason; next: inspect launcher diagnostics" >&2; exit 1; }
uv run --locked --package onetaskgraph-sdk onetaskgraph --help | grep -q 'Usage:' || { echo "Python SDK dependency did not supply the real command; next: inspect the SDK carrier dependency" >&2; exit 1; }
for bad_args in '' '1.2.3 extra' invalid; do
  read -r -a version_args <<< "$bad_args"
  if "$root/scripts/set-version.sh" "${version_args[@]}" 2>"$tmp/error"; then echo "version updater accepted invalid arguments: $bad_args; next: inspect argument validation" >&2; exit 1; fi
done
# shellcheck source=scripts/scratch-clone.sh
source "$root/scripts/scratch-clone.sh"
scratch_clone "$root" "$tmp/version-repo"
node -e 'const fs=require("fs"),f=process.argv[1],p=JSON.parse(fs.readFileSync(f));p.version="9.9.9";fs.writeFileSync(f,JSON.stringify(p,null,2)+"\n")' "$tmp/version-repo/npm/cli/package.json"
if "$tmp/version-repo/scripts/set-version.sh" --check 2>"$tmp/error"; then echo "version drift was accepted; next: inspect version checking" >&2; exit 1; fi
grep -q 'version drift found' "$tmp/error" || { echo "version-drift failure omitted recovery guidance; next: inspect version diagnostics" >&2; exit 1; }
git -C "$tmp/version-repo" restore npm/cli/package.json
"$tmp/version-repo/scripts/set-version.sh" 0.1.1
grep -q '^version = "0.1.1"' "$tmp/version-repo/Cargo.toml" || { echo "version updater missed the workspace manifest; next: inspect manifest mutation" >&2; exit 1; }
grep -q 'onetaskgraph-cli==0.1.1' "$tmp/version-repo/sdks/python/pyproject.toml" || { echo "version updater missed the Python CLI pin; next: inspect dependency mutation" >&2; exit 1; }
node -e 'const p=require(process.argv[1]); if(p.version!=="0.1.1" || Object.values(p.optionalDependencies).some(v=>v!=="0.1.1")) process.exit(1)' "$tmp/version-repo/npm/cli/package.json" || { echo "version updater missed npm metadata; next: inspect JSON mutation" >&2; exit 1; }
grep -q 'version = "0.1.1"' "$tmp/version-repo/Cargo.lock" || { echo "version updater missed Cargo.lock; next: inspect lock refresh" >&2; exit 1; }
grep -q 'version = "0.1.1"' "$tmp/version-repo/uv.lock" || { echo "version updater missed uv.lock; next: inspect lock refresh" >&2; exit 1; }
