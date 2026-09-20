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
readonly WORKFLOW=".github/workflows/release.yml"

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
body = re.search(r"(?ms)^      - name: Archive, checksum, and attach\n.*?^        run: \|\n(.*?)(?=^      - )", job.group(1))
if not body:
    print("check-release-packaging: the macOS job has no 'Archive, checksum, and attach' step with a `run: |` body", file=sys.stderr)
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

# A stand-in binary per target, and a `gh` that records each upload rather than making it.
readonly TREE="$scratch/tree"
mkdir -p "$TREE" "$scratch/bin" || fatal "could not create $TREE" "check the permissions of \$TMPDIR, then rerun"
(cd "$ROOT" && git ls-files -z -- npm/platforms | tar --null -T - -cf -) | tar -xf - -C "$TREE" || fatal \
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

version="$(cd "$ROOT" && python3 -c 'import json; print(json.load(open("npm/cli/package.json"))["version"])')" || fatal \
  "could not read the launcher's version out of npm/cli/package.json" "restore that manifest, then rerun"
tag="v$version"

failures=0
report() {
  failures=$((failures + 1))
  echo "check-release-packaging: $1" >&2
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
  if [ -f "$TREE/$asset" ] && ! tar -tzf "$TREE/$asset" | grep -qx onetaskgraph; then
    report "$asset does not carry a bare 'onetaskgraph' at its root, which is what the installer extracts"
  fi
  grep -qF -- "release upload $tag $asset $asset.sha256 --clobber" "$UPLOADS" 2>/dev/null || report \
    "the step never asked gh to upload $asset and its checksum to $tag with --clobber"
  carrier="$TREE/dist/carriers/$npm_platform/onetaskgraph-cli-$npm_platform-$version.tgz"
  [ -f "$carrier" ] || report "the step left no carrier at dist/carriers/$npm_platform/onetaskgraph-cli-$npm_platform-$version.tgz, which is the path and name scripts/publish-npm.sh reads after the carrier-$npm_platform artifact is downloaded"
  if [ -f "$carrier" ]; then
    tar -tzf "$carrier" | grep -qx "package/bin/onetaskgraph" || report "the $npm_platform carrier does not carry package/bin/onetaskgraph"
    packed_name="$(tar -xzOf "$carrier" package/package.json | python3 -c 'import json,sys; print(json.load(sys.stdin)["name"])')" || packed_name=""
    [ "$packed_name" = "@onetaskgraph/cli-$npm_platform" ] || report "the $npm_platform carrier's manifest names '$packed_name', expected @onetaskgraph/cli-$npm_platform"
  fi
done
if [ -f "$UPLOADS" ] && [ "$(wc -l < "$UPLOADS" | tr -d ' ')" -ne 2 ]; then
  report "the step made $(wc -l < "$UPLOADS" | tr -d ' ') gh calls over two targets, expected exactly two uploads:"
  sed 's/^/    /' "$UPLOADS" >&2
fi

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
  echo "check-release-packaging: repair the 'Archive, checksum, and attach' step of $WORKFLOW rather than these cases." >&2
  exit 1
fi
