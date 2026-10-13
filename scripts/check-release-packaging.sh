#!/usr/bin/env bash
# Drive the release workflow's packaging step, as written, over both macOS architectures.
#
# The step is a `run: |` block inside .github/workflows/release.yml, and a release is the
# only thing that runs it — so a defect in it was discoverable only by cutting a version.
# Folding both macOS targets into one job (#1991) turned that block into a loop over
# `TARGETS` pairs, and this reads the block out of the workflow and runs it here, against a
# stand-in binary per target and a `gh` that records what it was asked to upload, so what
# the fold packages is proven rather than read: one archive and one checksum uploaded per
# target, under the names scripts/install.sh downloads, and one carrier tarball per npm
# platform, in the directory and under the name scripts/publish-npm.sh reads.
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
readonly WORKFLOW=".github/workflows/release.yml"
python_bin=$(command -v python3 || true)
[ -n "$python_bin" ] || fatal "python3 is not on PATH" "install Python 3 and rerun this packaging proof"
case $python_bin in
  /* | ?:[\\/]*) ;;
  *) python_bin=$(cd "$(dirname "$python_bin")" && pwd)/$(basename "$python_bin") ;;
esac


for tool in npm python3 tar; do
  command -v "$tool" >/dev/null 2>&1 || fatal \
    "$tool is not on PATH, and the packaging step needs it" \
    "install $tool — 'just bootstrap' provisions the toolchain this repository needs — then rerun"
done

scratch="$(mktemp -d)" || fatal \
  "could not create the scratch tree this check packages in" \
  "check the permissions of \$TMPDIR and 'df -h' for free space, then rerun"
trap 'rm -rf "$scratch"' EXIT

# The step body of the macOS job, read out of the workflow rather than restated here, with
# its block indentation removed so bash can run it.
if ! (cd "$ROOT" && python3 - "$WORKFLOW" <<'PY'
import re
import sys
from pathlib import Path

workflow = Path(sys.argv[1]).read_text(encoding="utf-8")
job = re.search(r"(?ms)^  macos-assets-carriers-and-wheels:\n(.*?)(?=^  [a-z-]+:\n)", workflow)
if not job:
    print("check-release-packaging: the release workflow has no macos-assets-carriers-and-wheels job", file=sys.stderr)
    raise SystemExit(1)
body = re.search(r"(?ms)^      - name: Archive, checksum, and stage\n.*?^        run: \|\n(.*?)(?=^      - )", job.group(1))
if not body:
    print("check-release-packaging: the macOS job has no 'Archive, checksum, and stage' step with a `run: |` body", file=sys.stderr)
    raise SystemExit(1)
lines = body.group(1).splitlines()
margin = min((len(line) - len(line.lstrip()) for line in lines if line.strip()), default=0)
sys.stdout.reconfigure(newline="\n")
print("\n".join(line[margin:] for line in lines))
PY
) > "$scratch/package.sh" 2> "$scratch/package-error"; then
  fatal "the packaging step could not be read out of $WORKFLOW: $(cat "$scratch/package-error")" \
    "keep that step a 'run: |' block of the macOS job, which is what makes it drivable outside a release"
fi

# What the workflow hands the step: the job-level TARGETS and the step's own EXT and TAG.
targets="$(cd "$ROOT" && sed -n 's/^      TARGETS: \(.*\)$/\1/p' "$WORKFLOW" | head -n1)"
[ -n "$targets" ] || fatal "the macOS job declares no job-level TARGETS" "restore 'TARGETS: <target>=<npm> ...' under the job's env"
# Each pair is used as two path components below, so each is held to the grammar of a
# Rust target triple and an npm platform name before anything is created from it.
for pair in $targets; do
  [[ $pair =~ ^[a-z0-9_]+(-[a-z0-9_]+)+=[a-z0-9]+-[a-z0-9]+$ ]] || fatal \
    "the macOS job's TARGETS entry '$pair' is not <rust target triple>=<npm platform>" \
    "restore the job's TARGETS to pairs like x86_64-apple-darwin=darwin-x64"
done

# A stand-in binary per target, and a `gh` that records any premature upload.
readonly TREE="$scratch/tree"
mkdir -p "$TREE" "$scratch/bin" || fatal "could not create $TREE" "check the permissions of \$TMPDIR, then rerun"
copy_tracked_files "$ROOT" "$TREE" npm/platforms || fatal \
  "could not copy the carrier manifests into $TREE" "confirm 'git ls-files' answers in $ROOT, then rerun"
for pair in $targets; do
  target="${pair%%=*}"
  mkdir -p "$TREE/target/$target/release" || fatal "could not create the $target build directory" "check the permissions of \$TMPDIR, then rerun"
  printf 'stand-in for the %s binary\n' "$target" > "$TREE/target/$target/release/onetaskgraph"
done
readonly UPLOADS="$scratch/uploads"
cat > "$scratch/bin/gh" <<GH
#!/usr/bin/env bash
printf '%s\n' "\$*" >> "$UPLOADS"
GH
chmod +x "$scratch/bin/gh" || fatal "could not make the gh stand-in executable" "check the permissions of \$TMPDIR, then rerun"

version="$(cd "$ROOT" && python3 -c 'import json; print(json.load(open("npm/cli/package.json", encoding="utf-8"))["version"])')" || fatal \
  "could not read the launcher's version out of npm/cli/package.json" "restore that manifest, then rerun"
tag="v$version"

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

# 1. The step over both targets, exactly as the job runs it.
if ! (cd "$TREE" && PATH="$scratch/bin:$PATH" TARGETS="$targets" EXT=tar.gz TAG="$tag" GH_TOKEN=stand-in \
  bash "$scratch/package.sh") > "$scratch/package.log" 2>&1; then
  sed 's/^/    /' "$scratch/package.log" >&2
  report "the packaging step failed over TARGETS='$targets' (its output is above)"
fi
for pair in $targets; do
  target="${pair%%=*}"; npm_platform="${pair#*=}"
  asset="onetaskgraph-$tag-$target.tar.gz"
  [ -f "$TREE/$asset" ] || report "the step left no archive $asset, which is the name scripts/install.sh downloads for $target"
  [ -f "$TREE/$asset.sha256" ] || report "the step left no checksum $asset.sha256"
  if [ -f "$TREE/$asset" ]; then
    report_unless_archive_lists "$TREE/$asset" onetaskgraph \
      "$asset does not carry a bare 'onetaskgraph' at its root, which is what the installer extracts"
  fi
  cmp "$TREE/$asset" "$TREE/dist/assets/$asset" || report "the staged archive differs from $asset"
  cmp "$TREE/$asset.sha256" "$TREE/dist/assets/$asset.sha256" || report "the staged checksum differs from $asset.sha256"
  carrier="$TREE/dist/carriers/$npm_platform/onetaskgraph-cli-$npm_platform-$version.tgz"
  [ -f "$carrier" ] || report "the step left no carrier at dist/carriers/$npm_platform/onetaskgraph-cli-$npm_platform-$version.tgz, which is the path and name scripts/publish-npm.sh reads after the carrier-$npm_platform artifact is downloaded"
  if [ -f "$carrier" ]; then
    report_unless_archive_lists "$carrier" package/bin/onetaskgraph "the $npm_platform carrier does not carry package/bin/onetaskgraph"
    packed_name="$(tar -xzOf "$carrier" package/package.json | python3 -c 'import json,sys; print(json.load(sys.stdin)["name"])')" || packed_name=""
    [ "$packed_name" = "@onetaskgraph/cli-$npm_platform" ] || report "the $npm_platform carrier's manifest names '$packed_name', expected @onetaskgraph/cli-$npm_platform"
  fi
done
[ ! -f "$UPLOADS" ] || report "the builder published an asset before install verification"

# Foreign architectures are verified as artifacts; this Linux journey does not execute
# Darwin payloads. The production verifier compares their wheel/archive/npm byte hashes.
for pair in $targets; do
  target="${pair%%=*}"; npm_platform="${pair#*=}"
  if ! "$python_bin" - "$TREE" "$target" "$version" <<'PYWHEEL'
from pathlib import Path
import sys
import zipfile
root, target, version = sys.argv[1:]
root = Path(root)
arch = "arm64" if target.startswith("aarch64") else "x86_64"
platform = "macosx_11_0_" + arch
wheel_dir = root / "dist/wheels" / target
wheel_dir.mkdir(parents=True, exist_ok=True)
with zipfile.ZipFile(wheel_dir / f"onetaskgraph_cli-{version}-py3-none-{platform}.whl", "w") as wheel:
    wheel.writestr(f"onetaskgraph_cli-{version}.data/scripts/onetaskgraph", (root / "target" / target / "release/onetaskgraph").read_bytes())
    wheel.writestr(f"onetaskgraph_cli-{version}.dist-info/METADATA", f"Name: onetaskgraph-cli\nVersion: {version}\n")
    wheel.writestr(f"onetaskgraph_cli-{version}.dist-info/WHEEL", f"Tag: py3-none-{platform}\n")
PYWHEEL
  then report "could not stage the $target wheel fixture"; fi
  (cd "$TREE" && "$python_bin" "$ROOT/scripts/verify-release-products.py" --target "$target" --npm "$npm_platform" --tag "$tag" > /dev/null) || report "the production verifier refused the $target artifact contract"
done


# The verifier's trust boundary is portable: no foreign executable runs in these cases.
if ! "$python_bin" - "$ROOT" "$TREE" "$tag" <<'PYREFUSALS'
from pathlib import Path
import io
import json
import platform
import re
import subprocess
import sys
import tarfile
import zipfile
root, tree, tag = Path(sys.argv[1]), Path(sys.argv[2]), sys.argv[3]
version = tag[1:]
workflow = (root / '.github/workflows/release.yml').read_text()
windows, = [row for row in re.findall(r'target: ([a-z0-9_-]+), ext: ([a-z.]+), npm: ([a-z0-9-]+)', workflow) if row[1] == 'zip']
target, extension, npm = windows
binary = 'onetaskgraph.exe'
payload = b'foreign artifact contract payload; never executed'
canonical = tree / f'target/{target}/release/{binary}'
canonical.parent.mkdir(parents=True, exist_ok=True)
canonical.write_bytes(payload)
archive = tree / f'onetaskgraph-{tag}-{target}.zip'
with zipfile.ZipFile(archive, 'w') as packed:
    packed.writestr(binary, payload)
wheels = tree / f'dist/wheels/{target}'
wheels.mkdir(parents=True, exist_ok=True)
with zipfile.ZipFile(wheels / f'onetaskgraph_cli-{version}-py3-none-win_amd64.whl', 'w') as wheel:
    wheel.writestr(f'onetaskgraph_cli-{version}.data/scripts/{binary}', payload)
    wheel.writestr(f'onetaskgraph_cli-{version}.dist-info/METADATA', f'Name: onetaskgraph-cli\nVersion: {version}\n')
    wheel.writestr(f'onetaskgraph_cli-{version}.dist-info/WHEEL', 'Tag: py3-none-win_amd64\n')
carriers = tree / f'dist/carriers/{npm}'
carriers.mkdir(parents=True, exist_ok=True)
with tarfile.open(carriers / 'fixture.tgz', 'w:gz') as packed:
    for name, data in [('package/package.json', (root / f'npm/platforms/{npm}/package.json').read_bytes()), (f'package/bin/{binary}', payload)]:
        info = tarfile.TarInfo(name)
        info.size = len(data)
        packed.addfile(info, io.BytesIO(data))
command = [sys.executable, str(root / 'scripts/verify-release-products.py'), '--target', target, '--npm', npm, '--tag', tag, '--ext', extension]
def refused(args, reason):
    result = subprocess.run(args, cwd=tree, capture_output=True, text=True)
    if result.returncode == 0 or reason not in result.stderr:
        raise SystemExit(f'product boundary did not refuse {reason}: {result.stderr}; repair the production verifier')
for flags, reason in [(['--target', '../invalid'], 'invalid Rust target'), (['--npm', '../invalid'], 'invalid npm platform'), (['--tag', 'invalid'], 'invalid semantic release tag'), (['--ext', 'invalid'], 'invalid choice'), (['--npm', 'darwin-x64'], 'inconsistent target'), (['--ext', 'tar.gz'], 'inconsistent target')]:
    refused(command + flags, reason)
foreign = command
if platform.system() == 'Windows':
    pair = re.search(r'^      TARGETS: ([^ ]+)', workflow, re.M)[1]
    darwin_target, darwin_npm = pair.split('=')
    foreign = [sys.executable, str(root / 'scripts/verify-release-products.py'), '--target', darwin_target, '--npm', darwin_npm, '--tag', tag]
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
# Parse the real public verification result, then reject corrupted result envelopes.
sys.path.insert(0, str(root / 'scripts'))
from release_products import VerificationReport
verified = subprocess.run(command, cwd=tree, capture_output=True, text=True)
if verified.returncode:
    raise SystemExit(verified.stderr)
report = json.loads(verified.stdout)
VerificationReport.parse(verified.stdout)
for broken in [None, {}, {**report, 'payloadHashes': {}}, {**report, 'payloadHashes': {**report['payloadHashes'], 'binary': 'not-a-digest'}}]:
    try:
        VerificationReport.parse(json.dumps(broken))
    except ValueError:
        continue
    raise SystemExit('malformed verification report was accepted; repair its typed boundary parser')
if platform.system() == 'Linux':
    host_arm = platform.machine().lower() in ('aarch64', 'arm64')
    candidate, = [row for row in re.findall(r'target: ([a-z0-9_-]+), ext: ([a-z.]+), npm: ([a-z0-9-]+)', workflow) if row[0].endswith('unknown-linux-gnu') and row[0].startswith('aarch64') != host_arm]
    arm_target, arm_extension, arm_npm = candidate
    arm_payload = b'foreign architecture artifact; never executed'
    arm_binary = tree / f'target/{arm_target}/release/onetaskgraph'
    arm_binary.parent.mkdir(parents=True, exist_ok=True)
    arm_binary.write_bytes(arm_payload)
    with tarfile.open(tree / f'onetaskgraph-{tag}-{arm_target}.tar.gz', 'w:gz') as packed:
        info = tarfile.TarInfo('onetaskgraph')
        info.size = len(arm_payload)
        packed.addfile(info, io.BytesIO(arm_payload))
    arm_wheels = tree / f'dist/wheels/{arm_target}'
    arm_wheels.mkdir(parents=True, exist_ok=True)
    wheel_arch = 'aarch64' if arm_target.startswith('aarch64') else 'x86_64'
    wheel_platform = 'linux_' + wheel_arch
    with zipfile.ZipFile(arm_wheels / f'onetaskgraph_cli-{version}-py3-none-{wheel_platform}.whl', 'w') as packed:
        packed.writestr(f'onetaskgraph_cli-{version}.data/scripts/onetaskgraph', arm_payload)
        packed.writestr(f'onetaskgraph_cli-{version}.dist-info/METADATA', f'Name: onetaskgraph-cli\nVersion: {version}\n')
        packed.writestr(f'onetaskgraph_cli-{version}.dist-info/WHEEL', f'Tag: py3-none-{wheel_platform}\n')
    arm_carriers = tree / f'dist/carriers/{arm_npm}'
    arm_carriers.mkdir(parents=True, exist_ok=True)
    with tarfile.open(arm_carriers / 'fixture.tgz', 'w:gz') as packed:
        for name, data in [('package/package.json', (root / f'npm/platforms/{arm_npm}/package.json').read_bytes()), ('package/bin/onetaskgraph', arm_payload)]:
            info = tarfile.TarInfo(name)
            info.size = len(data)
            packed.addfile(info, io.BytesIO(data))
    refused([sys.executable, str(root / 'scripts/verify-release-products.py'), '--target', arm_target, '--npm', arm_npm, '--tag', tag, '--install'], 'cannot execute a foreign architecture')
elif platform.system() == 'Darwin' and platform.machine().lower() == 'x86_64':
    pair = next(pair for pair in re.search(r'^      TARGETS: (.*)$', workflow, re.M)[1].split() if pair.startswith('aarch64'))
    arm_target, arm_npm = pair.split('=')
    refused([sys.executable, str(root / 'scripts/verify-release-products.py'), '--target', arm_target, '--npm', arm_npm, '--tag', tag, '--install'], 'cannot execute a foreign architecture')
for entries in [[], [binary, 'nested/' + binary]]:
    with zipfile.ZipFile(archive, 'w') as packed:
        for entry in entries:
            packed.writestr(entry, payload)
    refused(command, 'archive must carry one CLI binary')
PYREFUSALS
then report "the production verifier accepted an invalid release product input"; fi

# 2. A tag the installer would refuse is refused here first, before anything is built.
rm -f "$UPLOADS"
if (cd "$TREE" && PATH="$scratch/bin:$PATH" TARGETS="$targets" EXT=tar.gz TAG="not-a-release" GH_TOKEN=stand-in \
  bash "$scratch/package.sh") > "$scratch/package.log" 2>&1; then
  report "the packaging step accepted the tag 'not-a-release'"
elif ! grep -qF "invalid release tag: not-a-release" "$scratch/package.log"; then
  report "the packaging step refused 'not-a-release' without naming the tag. It said:"
  sed 's/^/    /' "$scratch/package.log" >&2
fi
[ ! -f "$UPLOADS" ] || report "the packaging step uploaded something for an invalid tag"

if [ "$failures" -ne 0 ]; then
  echo "check-release-packaging: $failures finding(s)." >&2
  echo "check-release-packaging: repair the 'Archive, checksum, and stage' step of $WORKFLOW rather than these cases." >&2
  exit 1
fi
