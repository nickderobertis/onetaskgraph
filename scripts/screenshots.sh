#!/usr/bin/env bash
# Capture the terminal screenshots screencomp gates, galleries and posts to pull requests
# (see screencomp.toml and .github/workflows/visual-docs.yml).
#
# Every scene drives the REAL release `onetaskgraph` binary against the curated fixture in
# screenshots/fixture/ and renders exactly the bytes it wrote to a deterministic SVG with
# `freeze` and the VENDORED, pinned font. Nothing here rewrites a captured byte: the
# fixture is staged at a FIXED absolute path, so the paths the binary prints — a
# configuration document, a resolved `root`, a task's `location` — are real and identical
# on every machine. That matters more than it sounds: `config show` aligns its columns
# against the widest value it is printing, so a path rewritten afterwards would leave the
# origin column of two rows ninety characters left of every other row's. screenshots/AGENTS.md
# is the durable note; it says what each scene documents and why the fixture is curated.
#
# No network and no credential: the fixture configures `local-md` and `in-memory`, the two
# plugins compiled into every build that reach nothing, and never the two that reach a
# network — not even against a loopback fixture server, because a capture that binds a
# socket can fail for a reason that has nothing to do with the tool.
#
# Output (screencomp's capture contract):
#   $SHOTS_OUT/captures.json   index: {schema, shots:[{name,toggles,hash,image}]}
#   $SHOTS_OUT/<scene>.svg     one SVG per scene
# $SHOTS_OUT defaults to shots/current/<lane> (the reusable workflow exports it per lane).
# The SVGs are also copied to docs/screenshots/ (committed) for the README.
#
# The renderer comes from scripts/screenshots-freeze.sh, which owns the pin and the
# location; provision it with `just screenshots-tools`.
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Byte-determinism starts with the environment. The scenes render the real binary, and it
# reads `ONETASKGRAPH_*` settings ahead of the configuration document — so an exported one
# prints straight into the `config show` scene as the layer it won at, and one naming a
# source adds that source's rows to every listing. Clear every steering variable before
# setting the two this capture itself wants.
for _variable in $(compgen -e); do
  case "$_variable" in
  ONETASKGRAPH_*) unset "$_variable" ;;
  esac
done
unset _variable

# The one [capture].arches lane, read from screencomp.toml so the lane is declared in
# exactly one place — the same file the pre-push guard reads it from.
LANE="$(sed -n 's/^arches *= *\[ *"\([^"]*\)" *\].*/\1/p' screencomp.toml)"
if ! [[ "$LANE" =~ ^[A-Za-z0-9_]+$ ]]; then
  echo "screenshots: expected exactly one lane in [capture].arches of screencomp.toml; got: [${LANE:-none}]" >&2
  echo "screenshots: next: restore the single lane there, or give a second lane its own baseline and CI job" >&2
  exit 1
fi
readonly LANE
readonly SHOTS_OUT="${SHOTS_OUT:-shots/current/$LANE}"
# The reusable workflow and the guard both export this, so it is external input — and the
# capture removes it and everything under it below. It has to be a relative path inside
# `shots/` with no `..` segment before anything is deleted: an absolute or escaping value
# would name a directory this capture does not own.
case "$SHOTS_OUT" in
  shots/?*) ;;
  *)
    echo "screenshots: SHOTS_OUT is '$SHOTS_OUT', which is not a path under shots/ — and this capture removes what it names" >&2
    echo "screenshots: next: unset it to use shots/current/$LANE, or set it to a path under shots/ as the visual-docs workflow does" >&2
    exit 1
    ;;
esac
case "$SHOTS_OUT" in
  *..*)
    echo "screenshots: SHOTS_OUT is '$SHOTS_OUT', which walks out of shots/ with '..' — and this capture removes what it names" >&2
    echo "screenshots: next: name the lane directory outright, as shots/current/$LANE" >&2
    exit 1
    ;;
esac
readonly FONT="$ROOT/screenshots/fonts/JetBrainsMono-Regular.ttf"
readonly FIXTURE="$ROOT/screenshots/fixture"
readonly DOCS="$ROOT/docs/screenshots"
readonly BINARY="$ROOT/target/release/onetaskgraph"

# Where the fixture is staged, and the whole of the normalisation this capture needs. It is
# fixed rather than a mktemp directory because the binary prints absolute paths and aligns
# columns against them; it is short because a hundred-column path photographs no better
# than a hex-encoded id does. Everything the run could write — HOME, the XDG directories,
# TMPDIR — is redirected under it, so no ambient user configuration and no per-run
# temporary path can reach a shot.
readonly STAGE=/tmp/otg

FREEZE="$(bash "$ROOT/scripts/screenshots-freeze.sh" resolve)" || {
  echo "screenshots: the pinned renderer is not provisioned, so no scene can be rendered" >&2
  echo "screenshots: next: run 'just screenshots-tools', then re-run this capture" >&2
  exit 1
}
readonly FREEZE

if [ ! -r "$FONT" ]; then
  echo "screenshots: the vendored font is missing at $FONT, so freeze would fetch one over the network and the bytes would stop being reproducible" >&2
  echo "screenshots: next: restore it with 'git checkout -- screenshots/fonts'" >&2
  exit 1
fi

# The binary the scenes drive, release like a user would run. Rebuilt unless the caller
# says it has one; the guard and the workflow both let this build.
if [ -z "${SCREENSHOTS_NO_BUILD:-}" ] || [ ! -x "$BINARY" ]; then
  cargo build --release --locked --bin onetaskgraph >&2
fi
if [ ! -x "$BINARY" ]; then
  echo "screenshots: $BINARY was not built, so no scene can be captured" >&2
  echo "screenshots: next: run 'cargo build --release --locked --bin onetaskgraph' and read its diagnostic" >&2
  exit 1
fi

# Portable SHA-256 (Linux coreutils vs macOS/BSD).
sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# Stage the fixture at the fixed path. Removed first so a previous capture's tree — or a
# scene that wrote, should one ever be added — cannot carry into this one.
rm -rf "$STAGE"
mkdir -p "$STAGE/home/.config" "$STAGE/tmp"
cp -R "$FIXTURE/." "$STAGE/"

# The staged path has to BE the path the kernel reports, or every shot carrying one differs
# from the committed baseline: macOS resolves /tmp through a symlink to /private/tmp, and
# the binary canonicalizes what it prints. Refused rather than captured, because bytes no
# baseline can match are worse than no capture at all — CI's Linux container is where the
# baseline is made, and the pre-push guard says so when it skips.
staged="$(cd "$STAGE" && pwd -P)"
if [ "$staged" != "$STAGE" ]; then
  echo "screenshots: $STAGE resolves to $staged on this machine, so every shot carrying a path would differ from the committed baseline" >&2
  echo "screenshots: next: capture on a platform where that path is canonical (CI runs the container .github/workflows/visual-docs.yml names), or let CI's visual-docs workflow do it" >&2
  exit 1
fi

export HOME="$STAGE/home"
export XDG_CONFIG_HOME="$STAGE/home/.config"
export XDG_DATA_HOME="$STAGE/home/.local/share"
export XDG_STATE_HOME="$STAGE/home/.local/state"
export XDG_CACHE_HOME="$STAGE/home/.cache"
export TMPDIR="$STAGE/tmp"

# Fixed freeze flags. The vendored font, embedded into each SVG as base64, is what makes
# the output reproducible across machines and what keeps the file self-contained on GitHub
# and crates.io; everything else is fixed window styling. `--language ansi` is
# unconditional because freeze's content sniffing intermittently misreads plain aligned
# text as source, ignores --font.file and then hangs fetching a font over the network.
#
# One fixed window width for every scene, because the gallery and the README display each
# SVG at one width — a per-scene auto width made a narrow shot render huge and a wide one
# tiny. 93 columns clears the widest scene whose alignment matters (`config show`, 90) and
# `--wrap` folds the one genuinely over-wide scene, `sources list`, whose capability line
# is a single 157-column sentence; 93 is also the narrowest column at which freeze folds
# both of that scene's lines at a space rather than inside `orphan-tasks`.
# 843px = 30+30 padding + 93 * ~8.42px per character.
freeze_flags=(
  --language ansi
  --font.file "$FONT"
  --font.family "JetBrains Mono"
  --font.size 14
  --window
  --background "#0d1117"
  --padding "20,30"
  --margin 0
  --border.radius 8
  --width 843
  --wrap 93
)

rm -rf "$SHOTS_OUT"
mkdir -p "$SHOTS_OUT" "$DOCS"
captured="$(mktemp -d)"
trap 'rm -rf "$captured"' EXIT

# captures.json identity is `name + JSON.stringify(toggles)`; one record per scene, sorted
# at the end. No scene here varies along a dimension, so every toggle map is empty and
# screencomp.toml declares no `[[toggle]]`.
entries=()

# Drive one scene and render it. `directory` is the staged fixture the command runs in,
# which is what decides the configuration document it discovers.
#
# Every scene here is a successful invocation, and one that stops being successful is
# REFUSED rather than photographed: a shot of a diagnostic is still real output, but it
# would sail through the digest gate as a scene that merely changed. An empty capture is
# refused for the same reason — freeze renders one as a blank window.
scene() {
  local name="$1" directory="$2"
  shift 2
  local output="$captured/$name.txt" status=0
  (cd "$STAGE/$directory" && "$BINARY" "$@") >"$output" 2>&1 || status=$?
  if [ "$status" -ne 0 ]; then
    {
      echo "screenshots: scene '$name' exited $status, so what it wrote is a diagnostic rather than the surface this scene documents."
      echo "---- what it wrote ----"
      cat "$output"
    } >&2
    echo "screenshots: next: run 'onetaskgraph $*' in $STAGE/$directory and fix what it reports, or change the scene" >&2
    exit 1
  fi
  if [ ! -s "$output" ]; then
    echo "screenshots: scene '$name' produced no output, so there is nothing to render" >&2
    echo "screenshots: next: run 'onetaskgraph $*' in $STAGE/$directory and read what it does print" >&2
    exit 1
  fi
  # `< /dev/null`: freeze reads stdin whenever it is not a character device, so under a
  # piped stdin it ignores the file argument and renders "No input".
  if ! "$FREEZE" "$output" "${freeze_flags[@]}" -o "$SHOTS_OUT/$name.svg" </dev/null >&2; then
    echo "screenshots: the renderer failed on scene '$name', so that shot was not written" >&2
    echo "screenshots: next: read its diagnostic above; 'just screenshots-tools' reinstalls the pinned renderer" >&2
    exit 1
  fi
  entries+=("$name|{}|$(sha256 "$SHOTS_OUT/$name.svg")|$name.svg")
  # The committed copy: the same bytes, outside the gitignored shots/ tree.
  if ! cp "$SHOTS_OUT/$name.svg" "$DOCS/$name.svg"; then
    echo "screenshots: scene '$name' rendered but could not be copied to $DOCS, which is what the README embeds" >&2
    echo "screenshots: next: check the permissions of $DOCS, then re-run 'just screenshots'" >&2
    exit 1
  fi
}

# --- The scenes, each documenting the surface the README section it sits in explains. ----
#
# `sync/` is a checkout with two folders of Markdown — the plans, and where the team's
# tickets live. `board/` puts the same folder of plans beside a source that declares it
# cannot filter by label, which is what makes the plan block show both halves of the
# engine's bargain.

# The hero: the tool's main usage, interleaved across both sources in configured order.
scene task-list sync task list

# The aligned field block, the body, and the one comment the fixture's task carries.
scene task-show sync task show plans:T-1

# A same-source edge and one that leaves the source, reported by qualified id and kind.
scene task-deps sync task deps plans:T-1

# Every read, no write: the action each item would have got, and the reference figures.
scene task-copy-dry-run sync task copy plans:T-1 plans:T-2 --to work --dry-run

# The rows, then the per-source plan: pushed down to the folder, applied locally for the
# source that declares it cannot.
scene task-list-explain board task list --label chore --explain

# Each source with the capabilities it declares, which is what the plan above acts on.
scene sources-list board sources list

# Every effective setting and the layer it came from. The environment variable is set here
# rather than cleared with the rest, because the third column naming it — beside a flag,
# the document and a default in one shot — is the whole of what this scene documents.
export ONETASKGRAPH_DEFAULT_SOURCES=plans
scene config-show sync config show --page-size 50
unset ONETASKGRAPH_DEFAULT_SOURCES

# Write captures.json: shots sorted by identity, schema 1, trailing newline — the exact
# shape screencomp's classify, manifest and gallery read. Every field is safe ASCII.
{
  printf '{\n  "schema": 1,\n  "shots": [\n'
  IFS=$'\n' sorted=($(printf '%s\n' "${entries[@]}" | sort))
  unset IFS
  last=$((${#sorted[@]} - 1))
  for index in "${!sorted[@]}"; do
    IFS='|' read -r name toggles hash image <<<"${sorted[$index]}"
    comma=","
    [ "$index" -eq "$last" ] && comma=""
    printf '    {\n      "name": "%s",\n      "toggles": %s,\n      "hash": "%s",\n      "image": "%s"\n    }%s\n' \
      "$name" "$toggles" "$hash" "$image" "$comma"
  done
  printf '  ]\n}\n'
} >"$SHOTS_OUT/captures.json"

rm -rf "$STAGE"
echo "screenshots: wrote ${#entries[@]} shots to $SHOTS_OUT and docs/screenshots/" >&2
