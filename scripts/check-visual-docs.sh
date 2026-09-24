#!/usr/bin/env bash
# Reconcile everything the visual-docs adoption states twice.
#
# The capture is byte-gated, so what it renders cannot drift in silence. What CAN drift is
# everything around it that is written down in more than one place: the renderer pin, the
# arch lane, the two screencomp versions in the workflow, the scene inventory, and the
# images the README embeds. Each of those has one authoritative source in this tree, and
# this is the check that fails when a copy parts from it.
#
# It takes NO screenshot and needs neither screencomp nor the renderer installed, which is
# what lets it run in CI's check job like any other lint while the capture stays in its own
# workflow (screenshots/AGENTS.md records that split and why).
#
# Quiet on success. On failure it names the file and the edit.
#
# llmlint: ignore-file[code_lands_in_the_domain_that_owns_it] three commands of the `scripts` project enumerate that one directory; screenshots/AGENTS.md, "Where this machinery lives", is why.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)" && cd "$ROOT" || {
  echo "check-visual-docs: could not resolve and enter this repository's root from ${BASH_SOURCE[0]}, and every path below is relative to it" >&2
  echo "check-visual-docs: next: run it from a checkout of this repository, as 'nx run screenshots:lint' does" >&2
  exit 1
}
readonly ROOT

python3 - <<'PY'
import json
import re
import sys
from pathlib import Path

sys.dont_write_bytecode = True

problems = []


def read(path):
    try:
        return Path(path).read_text(encoding="utf-8")
    except OSError as error:
        problems.append(f"{path}: could not be read ({error}); restore it and re-run")
        return ""


config = read("screencomp.toml")
capture = read("scripts/screenshots.sh")
guard = read("scripts/screenshots-guard.sh")
bless = read("scripts/screenshots-bless.sh")
freeze = read("scripts/screenshots-freeze.sh")
workflow = read(".github/workflows/visual-docs.yml")
readme = read("README.md")
justfile = read("justfile")
ignored = read(".gitignore")

lanes = re.search(r"(?m)^arches\s*=\s*\[(.*?)\]", config)
lane_values = re.findall(r'"([^"]+)"', lanes.group(1)) if lanes else []
# Every DECLARED lane is held to the same charset the capture and the guard hold it to,
# because this check builds shots/baseline/<lane>.json out of it: a lane carrying a slash or
# a `..` would name a file outside that directory, and a lane this check cannot use is a
# lane they cannot either.
#
# None is discarded BEFORE the count, which is the whole of the difference. Filtering first
# read `["x86_64", "bad/lane"]` as one usable lane and said nothing — while the capture, the
# guard and the bless step, whose one shared pattern matches a single-entry array only, got
# no lane at all out of it and refused every push. The check that owns this contract must
# not be the one thing that accepts a file the tools it governs cannot read.
unusable = [value for value in lane_values if not re.fullmatch(r"[A-Za-z0-9_]+", value)]
if unusable or len(lane_values) != 1:
    problems.append(
        "screencomp.toml: [capture].arches must declare exactly one lane, named in "
        "letters, digits and underscores, because the pre-push guard classifies that lane "
        "on every host and only one baseline is committed; it declares "
        f"{lane_values or 'none'}"
        + (f", of which {unusable} cannot name a baseline file" if unusable else "")
        + ". A second lane needs its own baseline and its own CI job."
    )
lane = lane_values[0] if len(lane_values) == 1 and not unusable else "x86_64"

# Nothing may restate it — the workflow included, where the lane is a matrix screencomp
# reads from that same file and a prose copy is one nothing reconciles.
if lane and lane in workflow:
    problems.append(
        f".github/workflows/visual-docs.yml: names the lane {lane!r}. The one place it is "
        "declared is [capture].arches in screencomp.toml, which screencomp reads itself to "
        "fan out its matrix; a copy here is a second statement nothing keeps in step."
    )
for path, text in (
    ("scripts/screenshots.sh", capture),
    ("scripts/screenshots-guard.sh", guard),
    ("scripts/screenshots-bless.sh", bless),
):
    if "arches" not in text or "screencomp.toml" not in text:
        problems.append(
            f"{path}: does not read the lane out of [capture].arches in screencomp.toml. "
            "That file is the one place it is declared; reading it is what keeps the "
            "capture, the guard and the baseline naming the same lane."
        )
    if re.search(rf'(?m)^[^#]*["\']?{re.escape(lane)}["\']?\s*$', text):
        problems.append(
            f"{path}: names the lane {lane!r} outside a comment, which is a second "
            "spelling of screencomp.toml's [capture].arches; read it from there instead."
        )

# Where the review gallery goes, declared once in [guard].gallery — the key `screencomp
# init` scaffolds. The pre-push guard writes a directory there and .gitignore has to cover
# it, so both are reconciled against THIS declaration rather than against each other: a
# gallery moved here and nowhere else would otherwise leave a generated tree committed.
declared_gallery = re.search(r'(?m)^gallery\s*=\s*"([^"]*)"', config)
gallery = declared_gallery.group(1) if declared_gallery else ""
if not gallery or gallery.startswith("/") or ".." in gallery.split("/") or " " in gallery:
    problems.append(
        "screencomp.toml: [guard].gallery must declare a relative directory inside this "
        "tree, with no '..' segment and no space, because the pre-push guard writes the "
        "review gallery there and .gitignore has to ignore that directory; it declares "
        + (repr(declared_gallery.group(1)) if declared_gallery else "nothing")
        + "."
    )
    # Fall back to the scaffolded default, so the two reconciliations below still say
    # something useful rather than matching the empty string against every line.
    gallery = "shots/review"
# One line, outside a comment, naming both the key and the file: a mention in a comment is
# not a read, and the lane's own looser test — the substring anywhere in the text — is
# satisfied by the sentence explaining why the read is there.
if not re.search(r"(?m)^[^#\n]*\bgallery\b[^\n]*screencomp\.toml", guard):
    problems.append(
        "scripts/screenshots-guard.sh: no line reads the review gallery out of "
        "[guard].gallery in screencomp.toml. That file is the one place it is declared; "
        "reading it is what keeps the directory the guard writes and the directory "
        ".gitignore covers the same directory."
    )
if re.search(rf'(?m)^[^#]*["\']?{re.escape(gallery)}["\']?\s*$', guard):
    problems.append(
        f"scripts/screenshots-guard.sh: names the review gallery {gallery!r} outside a "
        "comment, which is a second spelling of screencomp.toml's [guard].gallery; read it "
        "from there instead."
    )

pinned = re.search(r'(?m)^readonly FREEZE_VERSION=([0-9]+\.[0-9]+\.[0-9]+)\s*$', freeze)
if not pinned:
    problems.append(
        "scripts/screenshots-freeze.sh: no `readonly FREEZE_VERSION=X.Y.Z` line, so the "
        "renderer pin has no authoritative source; restore it there."
    )
freeze_version = pinned.group(1) if pinned else ""
VERSIONED_FREEZE = re.compile(r"freeze[^\n]{0,40}?([0-9]+\.[0-9]+\.[0-9]+)")
for path in [
    "scripts/screenshots.sh",
    "scripts/screenshots-guard.sh",
    "scripts/screenshots-bless.sh",
    "justfile",
    ".githooks/pre-push",
    ".github/workflows/visual-docs.yml",
]:
    for line in read(path).splitlines():
        without_comment = line.split("#", 1)[0]
        if VERSIONED_FREEZE.search(without_comment):
            problems.append(
                f"{path}: states a freeze version ({line.strip()!r}). The pin lives once, "
                "in FREEZE_VERSION in scripts/screenshots-freeze.sh; reach the binary "
                "through that script instead."
            )

font = "screenshots/fonts/JetBrainsMono-Regular.ttf"
if not Path(font).is_file():
    problems.append(
        f"{font}: is missing. It is the other input the rendered bytes are a function of; "
        "without it freeze fetches a font over the network and the capture stops being "
        "reproducible."
    )
if not Path("screenshots/fonts/JetBrainsMono-OFL.txt").is_file():
    problems.append(
        "screenshots/fonts/JetBrainsMono-OFL.txt: is missing, and the vendored font is "
        "licensed under it; restore the licence beside the font."
    )
if font not in capture:
    problems.append(
        f"scripts/screenshots.sh: no longer names {font}, so the capture is not rendering "
        "with the vendored font this repository commits."
    )

scenes = re.findall(r"(?m)^scene ([a-z0-9-]+) ", capture)
if not scenes:
    problems.append(
        "scripts/screenshots.sh: declares no `scene <name> <dir> <argv...>` lines, so "
        "there is no scene inventory to reconcile."
    )
baseline_path = Path(f"shots/baseline/{lane}.json")
baseline_names = []
baseline_readable = False
if not baseline_path.is_file():
    problems.append(
        f"{baseline_path.as_posix()}: the committed digest baseline for the {lane} lane is "
        "missing. Capture and bless it with 'just screenshots-bless', then commit it."
    )
else:
    try:
        # Every entry is validated and a bad one REFUSES the manifest, rather than being
        # filtered out of the names below. A baseline carrying all seven expected names
        # and a malformed entry beside them would otherwise reconcile clean here while
        # being a document `screencomp classify` cannot gate against — which is the one
        # failure this reconciliation exists to catch before a push, not after it.
        baseline = json.loads(read(baseline_path))
        shots = baseline["shots"]
        if not isinstance(shots, list):
            raise TypeError(f"'shots' is {type(shots).__name__}, not a list")
        for position, shot in enumerate(shots):
            if not isinstance(shot, dict):
                raise TypeError(
                    f"shot {position} is {type(shot).__name__}, not an object"
                )
            if not isinstance(shot.get("name"), str):
                raise TypeError(f"shot {position} carries no string 'name'")
            baseline_names.append(shot["name"])
        baseline_readable = True
    except (ValueError, KeyError, TypeError) as error:
        baseline_names = []
        problems.append(
            f"{baseline_path.as_posix()}: is not a screencomp digest manifest ({error}). "
            "Re-bless it with 'just screenshots-bless' and commit the result; a file this "
            "cannot read is one classify cannot gate against either."
        )
# Only against a baseline this could read in full. One it could not is already refused just
# above, and naming every scene as missing from it would bury that with seven more lines.
for absent in sorted(set(scenes) - set(baseline_names)) if baseline_readable else []:
    problems.append(
        f"{baseline_path.as_posix()}: has no shot named {absent!r}, which "
        "scripts/screenshots.sh captures. Run 'just screenshots-bless' and commit the "
        "refreshed baseline."
    )
for stale in sorted(set(baseline_names) - set(scenes)) if baseline_readable else []:
    problems.append(
        f"{baseline_path.as_posix()}: carries a shot named {stale!r} that no scene in "
        "scripts/screenshots.sh captures any more. Re-bless the baseline."
    )

committed = sorted(path.name for path in Path("docs/screenshots").glob("*.svg"))
for scene in sorted(scenes):
    if f"{scene}.svg" not in committed:
        problems.append(
            f"docs/screenshots/{scene}.svg: is not committed, so the README cannot embed "
            "the scene the capture renders. Run 'just screenshots' and commit it."
        )

embedded = re.findall(r"!\[([^\]]*)\]\((docs/screenshots/[^)]+)\)", readme)
for alt, target in embedded:
    # The target is read off a document, so it is held to the one shape a committed capture
    # has before it is resolved against the filesystem: a `..` inside it would name a file
    # outside the directory the capture writes, and a picture from anywhere else is not one
    # the baseline gates.
    if not re.fullmatch(r"docs/screenshots/[a-z0-9-]+\.svg", target):
        problems.append(
            f"README.md: embeds {target}, which is not a `docs/screenshots/<scene>.svg` "
            "path. Every image in the README is a capture this baseline gates; one from "
            "anywhere else is a picture nothing keeps true."
        )
        continue
    if not Path(target).is_file():
        problems.append(
            f"README.md: embeds {target}, which is not a file in this tree. Every image "
            "the README references has to be committed, or the rendered page carries a "
            "broken picture."
        )
    if len(alt.strip()) < 30:
        problems.append(
            f"README.md: the image {target} has alt text {alt!r}, which is too short to "
            "describe what is IN the picture. Describe the picture, not the command."
        )
for orphan in committed:
    if not any(target == f"docs/screenshots/{orphan}" for _, target in embedded):
        problems.append(
            f"docs/screenshots/{orphan}: is committed but the README embeds it nowhere. "
            "Every committed capture sits in the section that explains the surface it "
            "shows, or it is not committed at all."
        )
if embedded:
    first_alt, first_target = embedded[0]
    if first_target != "docs/screenshots/task-list.svg":
        problems.append(
            "README.md: the first image is "
            f"{first_target}, where it has to be the hash-gated still of `task list` — "
            "the tool's main usage — immediately under the title."
        )
    title = readme.find("# onetaskgraph")
    if title == -1 or readme.find(f"]({first_target})") > readme.find("\n## "):
        problems.append(
            "README.md: the hero image does not sit immediately under the title, above "
            "the first section."
        )
else:
    problems.append(
        "README.md: embeds no capture at all. The hero is the first thing under the title."
    )

used = re.search(r"visual-docs-reusable\.yml@(v[0-9]+\.[0-9]+\.[0-9]+)", workflow)
passed = re.search(r"(?m)^\s*screencomp-version:\s*(v[0-9]+\.[0-9]+\.[0-9]+)\s*$", workflow)
if not used or not passed:
    problems.append(
        ".github/workflows/visual-docs.yml: must pin the reusable workflow with "
        "`uses: …@vX.Y.Z` AND pass the same version as `screencomp-version:`, so the "
        "workflow and the CLI it installs cannot disagree."
    )
elif used.group(1) != passed.group(1):
    problems.append(
        f".github/workflows/visual-docs.yml: pins the reusable workflow at "
        f"{used.group(1)} but installs screencomp {passed.group(1)}. They are the same "
        "version or the gate is comparing with a different tool than it runs."
    )
if not re.search(r"(?m)^\s*fail-on-drift:\s*true\s*$", workflow):
    problems.append(
        ".github/workflows/visual-docs.yml: must pass `fail-on-drift: true`. Without it a "
        "drifted capture is a warning, and the pictures can quietly stop being true."
    )
# The workflow captures by running the same scripts the recipes wrap, because the capture
# container carries no `just`. So the two spellings of "capture" are reconciled here: a
# recipe body that stopped naming one of these scripts, or a workflow step that reached for
# something else, would have CI and a person capturing differently.
capture_command = re.search(r"(?ms)^\s*capture-command:\s*\|(.*?)(?=\n\s*\w[\w-]*:|\Z)", workflow)
for script in ("scripts/screenshots-freeze.sh ensure", "scripts/screenshots.sh"):
    if not capture_command or script not in capture_command.group(1):
        problems.append(
            ".github/workflows/visual-docs.yml: its capture-command does not run "
            f"`{script}`, which is what `just screenshots-tools` and `just screenshots` "
            "run. CI and a person have to capture by the same route, or the baseline is "
            "gated against bytes nobody can reproduce locally."
        )
    if script not in justfile:
        problems.append(
            f"justfile: no recipe runs `{script}`. The recipes are the surface a person "
            "uses, and the workflow's capture-command runs the same scripts because its "
            "container has no `just`; one of the two moving alone parts them."
        )

container = re.search(r"(?m)^\s*container:\s*rust:([0-9]+\.[0-9]+\.[0-9]+)-", workflow)
channel = re.search(r'(?m)^channel\s*=\s*"([0-9.]+)"', read("rust-toolchain.toml"))
if not container:
    problems.append(
        ".github/workflows/visual-docs.yml: the capture container must be a pinned "
        "`rust:<X.Y.Z>-…` image, because the capture builds the real release binary."
    )
elif channel and container.group(1) != channel.group(1):
    problems.append(
        f".github/workflows/visual-docs.yml: captures in rust:{container.group(1)} while "
        f"rust-toolchain.toml pins {channel.group(1)}. rust-toolchain.toml is where that "
        "version is declared; bring the image tag to it."
    )

for tree in ("/shots/current/", "/shots/verify/", f"/{gallery.strip('/')}/"):
    if tree not in ignored:
        problems.append(
            f".gitignore: does not ignore {tree}. The digest baseline and the README "
            "images are committed; the regenerated capture trees and the review gallery "
            "are not."
        )
for kept in ("shots/baseline", "docs/screenshots"):
    if re.search(rf"(?m)^/?{re.escape(kept)}/?\s*$", ignored):
        problems.append(
            f".gitignore: ignores {kept}, which has to be committed — the baseline is the "
            "drift gate and the images are what the README embeds."
        )

# The recipe graph, read the way scripts/check-live-lane-selection.sh reads it.
recipes = {}
current = None
for line in justfile.splitlines():
    header = re.match(r"^([a-z][a-z0-9-]*)(\s+[^:]*)?:(.*)$", line)
    if header and not line.startswith((" ", "\t")):
        current = header.group(1)
        recipes[current] = {"deps": header.group(3).split(), "body": []}
    elif current and line.startswith((" ", "\t")):
        recipes[current]["body"].append(line)
    elif not line.strip():
        continue
    elif not line.startswith(("#", " ", "\t")):
        current = None


def reaches_capture(recipe, seen=None):
    seen = seen or set()
    if recipe in seen or recipe not in recipes:
        return False
    seen.add(recipe)
    if any("screenshots.sh" in line for line in recipes[recipe]["body"]):
        return True
    return any(reaches_capture(dependency, seen) for dependency in recipes[recipe]["deps"])


for recipe in ("check", "gate", "test", "lint", "typecheck", "coverage", "format-check"):
    if reaches_capture(recipe):
        problems.append(
            f"justfile: `just {recipe}` reaches scripts/screenshots.sh. Screenshots are "
            "informational: the capture belongs to `just screenshots`, the pre-push guard "
            "and .github/workflows/visual-docs.yml, and to nothing the gate runs."
        )
# Every Nx target of every project, by the COMMANDS it runs rather than by the files it
# declares as inputs — this project's lint names the capture script as an input on purpose,
# because a change to it has to re-run this check.
for project_file in sorted(Path(".").glob("*/project.json")) + sorted(
    Path(".").glob("*/*/project.json")
):
    try:
        targets = json.loads(read(project_file)).get("targets", {})
    except ValueError as error:
        problems.append(f"{project_file.as_posix()}: is not valid JSON ({error})")
        continue
    if not isinstance(targets, dict):
        problems.append(f'{project_file.as_posix()}: "targets" is not a JSON object')
        continue
    for name, target in targets.items():
        options = target.get("options", {}) if isinstance(target, dict) else {}
        options = options if isinstance(options, dict) else {}
        commands = options.get("commands", [])
        commands = list(commands) if isinstance(commands, list) else []
        if isinstance(options.get("command"), str):
            commands = [*commands, options["command"]]
        for command in commands:
            if isinstance(command, str) and "screenshots.sh" in command:
                problems.append(
                    f"{project_file.as_posix()}: target {name!r} runs the capture "
                    f"({command!r}). No Nx target may, because `nx affected` reaches every "
                    "one of them from the gate: the capture belongs to `just screenshots`, "
                    "the pre-push guard and .github/workflows/visual-docs.yml."
                )

if problems:
    print("check-visual-docs: the visual-docs setup has parted from its sources.", file=sys.stderr)
    for problem in problems:
        print(f"  {problem}", file=sys.stderr)
    print(
        "check-visual-docs: apply the edit each line above names, then re-run "
        "'./scripts/nx.sh run screenshots:lint'.",
        file=sys.stderr,
    )
    raise SystemExit(1)
PY
