#!/usr/bin/env bash
# Drive the packaging every CD build job runs, as written, over every released target.
#
# Only a release runs .github/workflows/release.yml, so this runs the packaging half of its
# build phase — scripts/release_cd.py's `package`, the same code `build` calls after maturin
# — offline, over stand-in wheels for all three platforms on whatever host this is, to find a
# packaging defect without cutting a version.
set -euo pipefail

fatal() {
  echo "check-release-packaging: $1" >&2
  echo "check-release-packaging: next: $2" >&2
  exit 2
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" || fatal \
  "could not resolve this repository's root from ${BASH_SOURCE[0]}" \
  "run the check from a checkout of this repository, as 'just distribution-check' does"
readonly ROOT
# The path is built from $ROOT at run time, so ShellCheck cannot follow it; the directive
# names the file it resolves to. Tested before it is sourced rather than guarded after:
# bash 3.2 ends the shell where `source` cannot find its file, so the handler after `||`
# never runs there and the reader is told nothing about what to restore.
# shellcheck source=scripts/scratch-clone.sh
if [ ! -r "$ROOT/scripts/scratch-clone.sh" ] || ! source "$ROOT/scripts/scratch-clone.sh"; then
  fatal "could not load $ROOT/scripts/scratch-clone.sh, which copies the tracked files below" \
    "restore it with 'git checkout -- scripts/scratch-clone.sh' and rerun"
fi
python_bin=$(command -v python3 || true)
[ -n "$python_bin" ] || fatal "python3 is not on PATH" "install Python 3 and rerun this packaging proof"
case $python_bin in
  /* | ?:[\\/]*) ;;
  *) python_bin=$(cd "$(dirname "$python_bin")" && pwd)/$(basename "$python_bin") ;;
esac


# The installer journey must use the build backend requirement declared by the product.
if ! "$python_bin" - "$ROOT" <<'PYREQUIREMENT'
import pathlib, re, sys, tomllib
root = pathlib.Path(sys.argv[1])
requires = tomllib.loads((root / 'pyproject.toml').read_text(encoding='utf-8'))['build-system']['requires']
script = (root / 'scripts/test-distribution.sh').read_text(encoding='utf-8')
selected = re.findall(r"uv tool run --from '([^']+)' maturin build", script)
if selected != requires:
    sys.exit('distribution maturin requirement disagrees with pyproject.toml')
PYREQUIREMENT
then
  fatal "distribution maturin requirement disagrees with pyproject.toml" "use the manifest build-system requirement in scripts/test-distribution.sh"
fi


for tool in npm python3 tar; do
  command -v "$tool" >/dev/null 2>&1 || fatal \
    "$tool is not on PATH, and the packaging step needs it" \
    "install $tool — 'just bootstrap' provisions the toolchain this repository needs — then rerun"
done

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree this check packages in" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

version="$(cd "$ROOT" && python3 -c 'import json; print(json.load(open("npm/cli/package.json", encoding="utf-8"))["version"])')" || fatal \
  "could not read the launcher's version out of npm/cli/package.json" "restore that manifest, then rerun"
tag="v$version"

# The tracked scripts and carrier manifests, so what runs is the tree under review, and one
# stand-in wheel per released target, named exactly as the plan says maturin names it.
readonly TREE="$scratch/tree"
mkdir -p "$TREE" || fatal "could not create $TREE" "check the permissions of \$TMPDIR, then rerun"
copy_tracked_files "$ROOT" "$TREE" scripts npm/platforms || fatal \
  "could not copy the scripts and carrier manifests into $TREE" "confirm 'git ls-files' answers in $ROOT, then rerun"
rows="$(cd "$TREE" && "$python_bin" - "$tag" <<'PY' | tr -d '\r'
import sys
sys.path.insert(0, "scripts")
from release_cd import plan
for platform in ("linux", "linux-arm64", "macos", "windows"):
    for artifact in plan(platform, sys.argv[1]):
        if artifact.kind == "wheel":
            print(platform, artifact.target, artifact.file)
PY
)" || fatal "could not read the release plan out of scripts/release_cd.py" "run 'python3 scripts/release_cd.py --platform linux plan --tag $tag' and fix what it reports"
[ -n "$rows" ] || fatal "the release plan names no wheel" "restore PLATFORM_TARGETS in scripts/release_products.py"
while read -r platform target wheel; do
  "$python_bin" - "$TREE" "$target" "$version" "$wheel" <<'PYWHEEL' || fatal "could not stage the $target stand-in wheel" "check the permissions of \$TMPDIR, then rerun"
from pathlib import Path
import sys
import zipfile
root, target, version, wheel = sys.argv[1:]
binary = "onetaskgraph.exe" if "windows" in target else "onetaskgraph"
tag = wheel.removesuffix(".whl").split("-", 2)[2]
directory = Path(root) / "dist/wheels" / target
directory.mkdir(parents=True, exist_ok=True)
with zipfile.ZipFile(directory / wheel, "w") as packed:
    packed.writestr(f"onetaskgraph_cli-{version}.data/scripts/{binary}", f"stand-in for the {target} binary\n")
    packed.writestr(f"onetaskgraph_cli-{version}.dist-info/METADATA", f"Name: onetaskgraph-cli\nVersion: {version}\n")
    packed.writestr(f"onetaskgraph_cli-{version}.dist-info/WHEEL", f"Tag: {tag}\n")
PYWHEEL
done <<< "$rows"

failures=0
report() {
  failures=$((failures + 1))
  echo "check-release-packaging: $1" >&2
}

# Report $3 unless the archive $1 lists the member $2, and report an archive tar cannot list at
# all; it returns success either way, because a failure is counted rather than returned. The
# listing is taken whole and its exit status checked before it is searched: piped into
# `grep -q`, which exits at its first match, the tar left writing to a closed pipe failed the
# pipeline under `pipefail` by timing alone and reported a carrier that held the binary as one
# that did not; and a listing that failed part way must not pass for one that found the member
# before it stopped.
report_unless_archive_lists() {
  local listing
  if ! listing="$(tar -tzf "$1" 2>&1)"; then
    report "tar could not list ${1#"$TREE"/}, so the step wrote an archive that is not a readable gzip tarball; repair how it creates it. tar said:"
    printf '%s\n' "$listing" | sed 's/^/    /' >&2
    return 0
  fi
  grep -qx -- "$2" <<<"$listing" || report "$3"
}

for platform in linux linux-arm64 macos windows; do
  if ! (cd "$TREE" && "$python_bin" scripts/release_cd.py --platform "$platform" package --tag "$tag" --dist "$TREE/dist") > "$scratch/package.log" 2>&1; then
    sed 's/^/    /' "$scratch/package.log" >&2
    report "the $platform packaging failed (its output is above)"
  fi
done
while read -r platform target wheel; do
  npm_platform="$(cd "$TREE" && "$python_bin" -c 'import sys; sys.path.insert(0, "scripts"); from release_products import PLATFORM_TARGETS; print(next(row.npm_platform for row in PLATFORM_TARGETS if row.target == sys.argv[1]))' "$target" | tr -d '\r')"
  ext=tar.gz binary=onetaskgraph
  [ "$platform" != windows ] || { ext=zip; binary=onetaskgraph.exe; }
  asset="onetaskgraph-$tag-$target.$ext"
  [ -f "$TREE/dist/assets/$asset" ] || report "the packaging left no archive $asset, which is the name scripts/install.sh downloads for $target"
  [ -f "$TREE/dist/assets/$asset.sha256" ] || report "the packaging left no checksum $asset.sha256"
  if [ -f "$TREE/dist/assets/$asset" ] && [ "$ext" = tar.gz ]; then
    report_unless_archive_lists "$TREE/dist/assets/$asset" onetaskgraph \
      "$asset does not carry a bare 'onetaskgraph' at its root, which is what the installer extracts"
  fi
  carrier="$TREE/dist/carriers/$npm_platform/onetaskgraph-cli-$npm_platform-$version.tgz"
  [ -f "$carrier" ] || report "the packaging left no carrier at dist/carriers/$npm_platform/onetaskgraph-cli-$npm_platform-$version.tgz, which is the path and name scripts/publish-npm.sh reads"
  if [ -f "$carrier" ]; then
    report_unless_archive_lists "$carrier" "package/bin/$binary" "the $npm_platform carrier does not carry package/bin/$binary"
    packed_name="$(tar -xzOf "$carrier" package/package.json | python3 -c 'import json,sys; print(json.load(sys.stdin)["name"])')" || packed_name=""
    [ "$packed_name" = "@onetaskgraph/cli-$npm_platform" ] || report "the $npm_platform carrier's manifest names '$packed_name', expected @onetaskgraph/cli-$npm_platform"
  fi
  # Foreign payloads are compared by wheel/archive/npm hashes; only the native product
  # journey installs and executes the host-compatible target.
  (cd "$TREE" && "$python_bin" scripts/verify-release-products.py --target "$target" --npm "$npm_platform" --tag "$tag" --ext "$ext" --dist "$TREE/dist" > /dev/null) || report "the production verifier refused the $target artifact contract"
done <<< "$rows"

# The verifier's trust boundary is portable: no foreign executable runs in these cases.
if ! "$python_bin" - "$ROOT" "$TREE" "$tag" <<'PYREFUSALS'
from pathlib import Path
import io
import json
import os
import platform
import re
import subprocess
import sys
import tarfile
import zipfile
root, tree, tag = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
version = tag[1:]
sys.path.insert(0, str(root / 'scripts'))
from release_products import PLATFORM_TARGETS
windows, = [row for row in PLATFORM_TARGETS if row.extension == 'zip']
target, extension, npm = windows.target, windows.extension, windows.npm_platform
binary = 'onetaskgraph.exe'
dist = tree / 'dist'
archive = dist / 'assets' / f'onetaskgraph-{tag}-{target}.zip'
canonical = tree / f'target/{target}/release/{binary}'
payload = canonical.read_bytes()
with zipfile.ZipFile(archive) as packed:
    entries = packed.infolist()
    if len(entries) != 1 or entries[0].filename != binary or packed.read(entries[0]) != payload:
        raise SystemExit('production Windows ZIP must contain only the canonical CLI at its root')
wheels = dist / f'wheels/{target}'
carriers = dist / f'carriers/{npm}'
command = [sys.executable, str(root / 'scripts/verify-release-products.py'), '--target', target, '--npm', npm, '--tag', tag, '--ext', extension, '--dist', str(dist)]
def refused(args, reason):
    result = subprocess.run(args, cwd=tree, capture_output=True, text=True)
    if result.returncode == 0 or reason not in result.stderr:
        raise SystemExit(f'product boundary did not refuse {reason}: {result.stderr}; repair the production verifier')
for flags, reason in [(['--target', '../invalid'], 'invalid Rust target'), (['--npm', '../invalid'], 'invalid npm platform'), (['--tag', 'invalid'], 'invalid semantic release tag'), (['--ext', 'invalid'], 'invalid choice'), (['--npm', 'darwin-arm64'], 'inconsistent target'), (['--ext', 'tar.gz'], 'inconsistent target')]:
    refused(command + flags, reason)
darwin, = [row for row in PLATFORM_TARGETS if row.platform == 'macos']
foreign = [sys.executable, str(root / 'scripts/verify-release-products.py'), '--target', darwin.target, '--npm', darwin.npm_platform, '--tag', tag, '--dist', str(dist)]
if platform.system() != 'Windows':
    refused(command + ['--install'], 'cannot execute a non-native release target')
if platform.system() != 'Darwin':
    refused(foreign + ['--install'], 'cannot execute a non-native release target')
wheel_path, = wheels.glob('*.whl')
original_wheel = wheel_path.read_bytes()
wheel_path.unlink()
refused(command, 'expected exactly one wheel')
wheel_path.write_bytes(original_wheel)
duplicate = wheels / 'duplicate.whl'
duplicate.write_bytes(original_wheel)
refused(command, 'expected exactly one wheel')
duplicate.unlink()
with zipfile.ZipFile(wheel_path) as packed:
    original_entries = [(entry, packed.read(entry.filename)) for entry in packed.infolist()]
for suffix, kind in [('/onetaskgraph.exe', 'wheel CLI entry'), ('/METADATA', 'wheel METADATA entry'), ('/WHEEL', 'wheel WHEEL entry')]:
    for duplicate_entry in [False, True]:
        with zipfile.ZipFile(wheel_path, 'w') as packed:
            for entry, data in original_entries:
                if not entry.filename.endswith(suffix) or duplicate_entry:
                    packed.writestr(entry, data)
                if entry.filename.endswith(suffix) and duplicate_entry:
                    packed.writestr('duplicate' + entry.filename, data)
        refused(command, 'expected exactly one ' + kind)
    wheel_path.write_bytes(original_wheel)
with zipfile.ZipFile(wheel_path, 'w') as packed:
    for entry, data in original_entries:
        packed.writestr(entry, f'Name: foreign-cli\nVersion: {version}\n'.encode() if entry.filename.endswith('/METADATA') else data)
refused(command, 'wheel identity or version mismatch')
wheel_path.write_bytes(original_wheel)
wrong_arch = wheel_path.with_name(wheel_path.name.replace('win_amd64', 'win_arm64'))
wheel_path.rename(wrong_arch)
with zipfile.ZipFile(wrong_arch, 'w') as packed:
    for entry, data in original_entries:
        packed.writestr(entry, b'Tag: py3-none-win_arm64\n' if entry.filename.endswith('/WHEEL') else data)
refused(command, 'wheel architecture or OS mismatch')
wrong_arch.unlink()
wheel_path.write_bytes(original_wheel)
malformed_name = wheel_path.with_name(f'onetaskgraph_cli-{version}-broken.whl')
wheel_path.rename(malformed_name)
refused(command, 'invalid wheel filename tags')
malformed_name.rename(wheel_path)
carrier_path, = carriers.glob('*.tgz')
original_carrier = carrier_path.read_bytes()
carrier_path.unlink()
refused(command, 'expected exactly one npm carrier')
carrier_path.write_bytes(original_carrier)
extra_carrier = carriers / 'duplicate.tgz'
extra_carrier.write_bytes(original_carrier)
refused(command, 'expected exactly one npm carrier')
extra_carrier.unlink()

with tarfile.open(carrier_path) as packed:
    carrier_members = [(info, packed.extractfile(info).read() if info.isfile() else None) for info in packed.getmembers()]
for key, bad in [('name', None), ('version', 1), ('os', 'windows'), ('cpu', [1])]:
    with tarfile.open(carrier_path, 'w:gz') as packed:
        for info, data in carrier_members:
            if info.name == 'package/package.json':
                manifest = json.loads(data)
                manifest[key] = bad
                data = json.dumps(manifest).encode()
                info.size = len(data)
            packed.addfile(info, io.BytesIO(data) if data is not None else None)
    refused(command, 'invalid carrier identity fields' if key in ('name', 'version') else 'invalid carrier platform fields')
carrier_path.write_bytes(original_carrier)
sys.path.insert(0, str(root / 'scripts'))
from release_products import VerificationReport
verified = subprocess.run(command, cwd=tree, capture_output=True, text=True)
if verified.returncode:
    raise SystemExit(verified.stderr)
report = json.loads(verified.stdout)
VerificationReport.parse(verified.stdout)
for broken in [None, {}, {**report, 'target': '../invalid'}, {**report, 'tag': 'invalid'}, {**report, 'payloadHashes': {}}, {**report, 'payloadHashes': {**report['payloadHashes'], 'binary': 'not-a-digest'}}]:
    try:
        VerificationReport.parse(json.dumps(broken))
    except ValueError:
        continue
    raise SystemExit('malformed verification report was accepted; repair its typed boundary parser')
# A Linux host asked to install the other architecture's Linux release.
if platform.system() == 'Linux':
    host_arm = platform.machine().lower() in ('aarch64', 'arm64')
    linux, = [row for row in PLATFORM_TARGETS if row.platform == ('linux' if host_arm else 'linux-arm64')]
    other_target, other_extension, other_npm = linux.target, linux.extension, linux.npm_platform
    other_payload = b'foreign architecture artifact; never executed'
    other_binary = tree / f'target/{other_target}/release/onetaskgraph'
    other_binary.parent.mkdir(parents=True, exist_ok=True)
    other_binary.write_bytes(other_payload)
    with tarfile.open(dist / 'assets' / f'onetaskgraph-{tag}-{other_target}.tar.gz', 'w:gz') as packed:
        info = tarfile.TarInfo('onetaskgraph')
        info.size = len(other_payload)
        packed.addfile(info, io.BytesIO(other_payload))
    other_wheels = dist / f'wheels/{other_target}'
    for stale in other_wheels.glob('*.whl'):
        stale.unlink()
    other_wheels.mkdir(parents=True, exist_ok=True)
    wheel_arch = 'aarch64' if other_target.startswith('aarch64') else 'x86_64'
    wheel_platform = 'linux_' + wheel_arch
    with zipfile.ZipFile(other_wheels / f'onetaskgraph_cli-{version}-py3-none-{wheel_platform}.whl', 'w') as packed:
        packed.writestr(f'onetaskgraph_cli-{version}.data/scripts/onetaskgraph', other_payload)
        packed.writestr(f'onetaskgraph_cli-{version}.dist-info/METADATA', f'Name: onetaskgraph-cli\nVersion: {version}\n')
        packed.writestr(f'onetaskgraph_cli-{version}.dist-info/WHEEL', f'Tag: py3-none-{wheel_platform}\n')
    other_carriers = dist / f'carriers/{other_npm}'
    for stale in other_carriers.glob('*.tgz'):
        stale.unlink()
    other_carriers.mkdir(parents=True, exist_ok=True)
    with tarfile.open(other_carriers / 'fixture.tgz', 'w:gz') as packed:
        for name, data in [('package/package.json', (root / f'npm/platforms/{other_npm}/package.json').read_bytes()), ('package/bin/onetaskgraph', other_payload)]:
            info = tarfile.TarInfo(name)
            info.size = len(data)
            packed.addfile(info, io.BytesIO(data))
    refused([sys.executable, str(root / 'scripts/verify-release-products.py'), '--target', other_target, '--npm', other_npm, '--tag', tag, '--dist', str(dist), '--install'], 'cannot execute a foreign architecture')
archive_bytes = archive.read_bytes()
archive.write_bytes(b'corrupt ZIP')
refused(command, 'File is not a zip file')
archive.write_bytes(archive_bytes)
for entries in [[], [binary, 'nested/' + binary]]:
    with zipfile.ZipFile(archive, 'w') as packed:
        for entry in entries:
            packed.writestr(entry, payload)
    refused(command, 'expected exactly one archive CLI entry')
with zipfile.ZipFile(archive, 'w') as packed:
    packed.writestr('nested/' + binary, payload)
refused(command, 'archive must contain only the root regular CLI file')
with zipfile.ZipFile(archive, 'w') as packed:
    entry = zipfile.ZipInfo(binary)
    entry.create_system = 3
    entry.external_attr = 0o120777 << 16
    packed.writestr(entry, payload)
refused(command, 'archive must contain only the root regular CLI file')
PYREFUSALS
then report "the production verifier accepted an invalid release product input"; fi

# 2. A tag the installer would refuse is refused here first, before anything is packaged.
if (cd "$TREE" && "$python_bin" scripts/release_cd.py --platform macos package --tag not-a-release --dist "$scratch/invalid-tag") > "$scratch/package.log" 2>&1; then
  report "the packaging accepted the tag 'not-a-release'"
elif ! grep -qF "invalid semantic release tag 'not-a-release'" "$scratch/package.log"; then
  report "the packaging refused 'not-a-release' without naming the tag. It said:"
  sed 's/^/    /' "$scratch/package.log" >&2
fi
[ ! -e "$scratch/invalid-tag" ] || report "the packaging wrote something for an invalid tag"

if [ "$failures" -ne 0 ]; then
  echo "check-release-packaging: $failures finding(s)." >&2
  echo "check-release-packaging: repair package_target in scripts/release_cd.py, or the verifier it names, rather than these cases." >&2
  exit 1
fi
