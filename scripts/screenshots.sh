#!/usr/bin/env bash
# Capture the terminal screenshots screencomp gates, galleries and posts to pull requests
# (see screencomp.toml and .github/workflows/visual-docs.yml).
#
# Every scene drives the REAL release `onetaskgraph` binary against the curated fixture in
# screenshots/fixture/ and renders exactly the bytes it wrote. screenshots/AGENTS.md is the
# durable note: what each scene documents, why the fixture is curated, what the capture is
# pinned to, why it configures only the two plugins that reach nothing, and why staging the
# fixture at a FIXED absolute path is the whole of the normalisation.
#
# Output (screencomp's capture contract):
#   $SHOTS_OUT/captures.json   index: {schema, shots:[{name,toggles,hash,image}]}
#   $SHOTS_OUT/<scene>.svg     one SVG per scene
# $SHOTS_OUT defaults to shots/current/<lane> (the reusable workflow exports it per lane).
# The SVGs are also copied to docs/screenshots/ (committed) for the README.
#
# The renderer comes from scripts/screenshots-freeze.sh, which owns the pin and the
# location; provision it with `just screenshots-tools`.
#
# llmlint: ignore-file[code_lands_in_the_domain_that_owns_it] three commands of the `scripts` project enumerate that one directory; screenshots/AGENTS.md, "Where this machinery lives", is why.
set -euo pipefail

# `pwd -P` rather than `pwd`, because ROOT is one SIDE of the containment comparison below
# and the other side is resolved. A logical `pwd` keeps whatever symlink the caller walked
# in through: on the macOS runner $TMPDIR is /var/folders/…, itself a symlink to
# /private/var/folders/…, so a clone there had `$ROOT/shots/`* compared against a
# `pwd -P` that had already resolved that ancestor — and the capture refused its own lane
# directory, naming two spellings of the SAME path, before any scenario driving it could
# reach what it was about. Linux is where the two spellings coincide, which is why nothing
# saw it there. Both sides physical is the fix; the comparison is no weaker for it, because
# a symlink that redirects out of the clone still resolves somewhere `$ROOT/shots/` is not a
# prefix of.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)" && cd "$ROOT" || {
  echo "screenshots: could not resolve and enter this repository's root from ${BASH_SOURCE[0]}, and every path below is relative to it" >&2
  echo "screenshots: next: run it from a checkout of this repository, as 'just screenshots' does" >&2
  exit 1
}
readonly ROOT

# Byte-determinism starts with the environment. The scenes render the real binary, and it
# reads `ONETASKGRAPH_*` settings ahead of the configuration document — so an exported one
# prints straight into the `config show` scene as the layer it won at, and one naming a
# source adds that source's rows to every listing. Clear every steering variable before
# setting the two this capture itself wants.
#
# `ONETASKGRAPH_TOOLS_HOME` is spelled like one of those settings and is not one: it names
# where scripts/screenshots-freeze.sh provisions the pinned renderer, and the binary never
# reads it for anything — it reaches the environment layer only because that layer takes the
# whole `ONETASKGRAPH_` prefix. So it is kept here, by name, and handed to the resolver
# alone; the binary's own environment still carries no `ONETASKGRAPH_` variable this capture
# did not set. Swept and not kept, the capture resolved the renderer from the AMBIENT cache
# home whatever the caller named — which passes on a machine that happens to have one
# provisioned there and refuses on one that does not, so `check (ubuntu-latest)` failed on
# four of scripts/check-visual-tools.sh's cases while every local run passed.
TOOLS_HOME="${ONETASKGRAPH_TOOLS_HOME:-}"
readonly TOOLS_HOME
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

# The lexical checks above are not enough on their own: a symlink at any component under
# `shots/` redirects this path out of the clone without a `..` or a leading slash anywhere
# in it, and only resolving it can see that. So it is resolved BEFORE anything is created.
#
# What is resolved is the deepest ancestor of SHOTS_OUT that already exists, because a path
# that does not exist yet cannot be resolved at all — and resolving it after creating it,
# which is what this did until it was watched, has `mkdir -p` follow the symlink and make a
# directory outside the clone in the moment before the refusal. An empty directory is a far
# smaller harm than the removal below, but it is a write outside this tree that nobody
# asked for, and the containment check exists to make those impossible rather than brief.
#
# All of it before the renderer is resolved and the binary is built, because deciding where
# this capture may write costs nothing and a release build costs minutes.
existing="$SHOTS_OUT"
while [ ! -d "$existing" ]; do
  parent="$(dirname "$existing")"
  if [ "$parent" = "$existing" ]; then
    break
  fi
  existing="$parent"
done
# `$ROOT/shots/`* rather than `?*` here: this is an ANCESTOR, so `shots` itself is the
# legitimate answer for a lane directory that does not exist yet. The resolution of the
# full path below is the one that refuses `shots` as a destination.
resolved_existing="$(cd "$existing" 2>/dev/null && pwd -P)" || resolved_existing=""
case "${resolved_existing}/" in
  "$ROOT/shots/"*) ;;
  *)
    echo "screenshots: SHOTS_OUT ('$SHOTS_OUT') has $existing as its deepest existing directory, which resolves to ${resolved_existing:-nothing readable} — outside $ROOT/shots. Nothing was created." >&2
    echo "screenshots: next: take the symlink out of that path, or point SHOTS_OUT at a lane directory under shots/, as shots/current/$LANE" >&2
    exit 1
    ;;
esac
mkdir -p "$SHOTS_OUT" "$DOCS" || {
  echo "screenshots: could not create $SHOTS_OUT and $DOCS, which is where this capture writes" >&2
  echo "screenshots: next: check the permissions of $ROOT/shots and $DOCS, then re-run 'just screenshots'" >&2
  exit 1
}
resolved_out="$(cd "$SHOTS_OUT" && pwd -P)"
# `?*` rather than `*`: one character after the slash at least, so `shots` ITSELF is not a
# value this accepts — `shots/.` resolves there, and what follows would take the committed
# baseline with it.
case "$resolved_out/" in
  "$ROOT/shots/"?*) ;;
  *)
    echo "screenshots: SHOTS_OUT ('$SHOTS_OUT') resolves to $resolved_out, which is not a directory INSIDE $ROOT/shots — and this capture removes what it names" >&2
    echo "screenshots: next: take the symlink out of that path, or point SHOTS_OUT at a lane directory under shots/, as shots/current/$LANE" >&2
    exit 1
    ;;
esac
rm -rf "$resolved_out" && mkdir -p "$SHOTS_OUT" || {
  echo "screenshots: could not clear and recreate $resolved_out for this capture" >&2
  echo "screenshots: next: check what holds it open and the permissions of $ROOT/shots, then re-run 'just screenshots'" >&2
  exit 1
}
FREEZE="$(ONETASKGRAPH_TOOLS_HOME="$TOOLS_HOME" \
  bash "$ROOT/scripts/screenshots-freeze.sh" resolve)" || {
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

# The binary the scenes drive, release like a user would run. The guard and the workflow
# both let this build; SCREENSHOTS_NO_BUILD means what it says — no build here, and a
# refusal rather than one behind the caller's back when there is nothing to drive.
#
# Read as a boolean rather than by non-emptiness: `SCREENSHOTS_NO_BUILD=0` asks for a build,
# and a capture that answered it with none would refuse on a machine with no binary — the
# opposite of what the caller wrote. A value that is neither is a typo the caller meant
# something by, so it is named rather than guessed at in either direction.
case "$(printf '%s' "${SCREENSHOTS_NO_BUILD:-}" | tr '[:upper:]' '[:lower:]')" in
  '' | 0 | false | no | off) no_build="" ;;
  1 | true | yes | on) no_build=yes ;;
  *)
    echo "screenshots: SCREENSHOTS_NO_BUILD is '${SCREENSHOTS_NO_BUILD}', which is neither on (1, true, yes, on) nor off (0, false, no, off)" >&2
    echo "screenshots: next: set it to one of those spellings, or unset it and let this capture build the binary" >&2
    exit 1
    ;;
esac
readonly no_build
if [ -n "$no_build" ]; then
  if [ ! -x "$BINARY" ]; then
    echo "screenshots: SCREENSHOTS_NO_BUILD is set and there is no binary at $BINARY to capture" >&2
    echo "screenshots: next: build it with 'cargo build --release --locked --bin onetaskgraph', or unset SCREENSHOTS_NO_BUILD and let this capture build it" >&2
    exit 1
  fi
elif ! cargo build --release --locked --bin onetaskgraph >&2; then
  echo "screenshots: the release binary the scenes drive did not build, so nothing was captured" >&2
  echo "screenshots: next: read cargo's diagnostic above and re-run 'just screenshots'" >&2
  exit 1
fi

# Portable SHA-256 (Linux coreutils vs macOS/BSD), resolved once so a machine with neither
# is told so rather than meeting `command not found` halfway through a capture.
if command -v sha256sum >/dev/null 2>&1; then
  digest_command=(sha256sum)
elif command -v shasum >/dev/null 2>&1; then
  digest_command=(shasum -a 256)
else
  echo "screenshots: neither sha256sum nor shasum is on PATH, so the hashes screencomp gates on cannot be computed" >&2
  echo "screenshots: next: install coreutils (or perl's shasum), then re-run 'just screenshots'" >&2
  exit 1
fi

# The digest goes straight into captures.json, so what the tool answered is held to the
# shape a SHA-256 has: anything else would be written into the index screencomp reads.
sha256() {
  local answer
  answer="$("${digest_command[@]}" "$1" | cut -d' ' -f1)"
  if ! [[ "$answer" =~ ^[0-9a-f]{64}$ ]]; then
    echo "screenshots: hashing $1 answered '${answer}', which is not a SHA-256 digest" >&2
    echo "screenshots: next: check what '${digest_command[*]}' is on this PATH, then re-run 'just screenshots'" >&2
    exit 1
  fi
  printf '%s\n' "$answer"
}

# Stage the fixture at the fixed path. Removed first so a previous capture's tree — or a
# scene that wrote, should one ever be added — cannot carry into this one.
rm -rf "$STAGE" && mkdir -p "$STAGE/home/.config" "$STAGE/tmp" \
  && cp -R "$FIXTURE/." "$STAGE/" || {
  echo "screenshots: could not stage $FIXTURE at $STAGE, which is where every scene runs" >&2
  echo "screenshots: next: check what holds $STAGE open and the free space on that filesystem, then re-run 'just screenshots'" >&2
  exit 1
}

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

captured="$(mktemp -d)" || {
  echo "screenshots: could not create the temporary directory each scene's output is held in" >&2
  echo "screenshots: next: check the permissions of \$TMPDIR and 'df -h' for free space, then re-run 'just screenshots'" >&2
  exit 1
}
trap 'rm -rf "$captured"' EXIT

# captures.json identity is `name + JSON.stringify(toggles)`; one record per scene, sorted
# at the end. No scene here varies along a dimension, so every toggle map is empty and
# screencomp.toml declares no `[[toggle]]`.
entries=()

# The binary discovers its configuration by the directory it is run in, which is the whole
# of how a scene chooses between the two staged fixtures: they differ in nothing else.
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

# What each one documents, and why it earns a place in the README, is screenshots/AGENTS.md.
# What is needed to READ the calls is which fixture each runs in: `sync/` is a checkout with
# two folders of Markdown, the plans and where the team's tickets live; `board/` puts that
# same folder of plans beside a source declaring it cannot filter by label, which is what
# makes a plan block show both halves of the engine's bargain.
scene task-list sync task list
scene task-show sync task show plans:T-1
scene task-deps sync task deps plans:T-1
scene task-copy-dry-run sync task copy plans:T-1 plans:T-2 --to work --dry-run
scene task-list-explain board task list --label chore --explain
scene sources-list board sources list

# The variable is set here rather than cleared with the rest, because the third column
# naming it — beside a flag, the document and a default in one shot — is the whole of what
# this scene documents.
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
} >"$SHOTS_OUT/captures.json" || {
  echo "screenshots: the shots rendered but the index at $SHOTS_OUT/captures.json could not be written, and screencomp reads the capture through it" >&2
  echo "screenshots: next: check the permissions of $SHOTS_OUT and 'df -h' for free space, then re-run 'just screenshots'" >&2
  exit 1
}

rm -rf "$STAGE" || {
  echo "screenshots: every shot was written, but the staged fixture at $STAGE could not be removed — the next capture stages over it, so a file left there can reach a shot" >&2
  echo "screenshots: next: remove $STAGE by hand, then re-run 'just screenshots' and check the shots it writes" >&2
  exit 1
}
# Quiet on success. What it wrote is $SHOTS_OUT and docs/screenshots/, which is where the
# caller pointed it; a refusal above is the only thing this script has to say.
