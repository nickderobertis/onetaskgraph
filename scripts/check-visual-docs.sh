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
set -euo pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

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

# --- The lane, declared in screencomp.toml and read from there by everything else -------
lanes = re.search(r"(?m)^arches\s*=\s*\[(.*?)\]", config)
lane_values = re.findall(r'"([^"]+)"', lanes.group(1)) if lanes else []
if len(lane_values) != 1:
    problems.append(
        "screencomp.toml: [capture].arches must declare exactly one lane, because the "
        "pre-push guard classifies that lane on every host and only one baseline is "
        f"committed; it declares {lane_values or 'none'}. A second lane needs its own "
        "baseline and its own CI job."
    )
lane = lane_values[0] if lane_values else "x86_64"

# Nothing may restate it. Each of these reads it out of screencomp.toml with the same sed.
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

# --- The renderer pin, stated once in the script that owns which freeze this repo runs --
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

# --- The vendored font, named once and present ------------------------------------------
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

# --- The scenes: the capture, the committed baseline and docs/screenshots agree ---------
scenes = re.findall(r"(?m)^scene ([a-z0-9-]+) ", capture)
if not scenes:
    problems.append(
        "scripts/screenshots.sh: declares no `scene <name> <dir> <argv...>` lines, so "
        "there is no scene inventory to reconcile."
    )
baseline_path = Path(f"shots/baseline/{lane}.json")
baseline_names = []
if not baseline_path.is_file():
    problems.append(
        f"{baseline_path.as_posix()}: the committed digest baseline for the {lane} lane is "
        "missing. Capture and bless it with 'just screenshots-bless', then commit it."
    )
else:
    baseline_names = re.findall(r'"name"\s*:\s*"([^"]+)"', read(baseline_path))
for absent in sorted(set(scenes) - set(baseline_names)):
    problems.append(
        f"{baseline_path.as_posix()}: has no shot named {absent!r}, which "
        "scripts/screenshots.sh captures. Run 'just screenshots-bless' and commit the "
        "refreshed baseline."
    )
for stale in sorted(set(baseline_names) - set(scenes)):
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

# --- The README: the hero first, every image committed, every image placed --------------
embedded = re.findall(r"!\[([^\]]*)\]\((docs/screenshots/[^)]+)\)", readme)
for alt, target in embedded:
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

# --- The two screencomp versions in the workflow, and the container's toolchain ----------
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

# --- What is committed and what is regenerated ------------------------------------------
for tree in ("/shots/current/", "/shots/verify/", "/shots/review/"):
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

# --- No capture is reachable from the gate ----------------------------------------------
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
    for name, target in targets.items():
        options = target.get("options", {}) if isinstance(target, dict) else {}
        commands = options.get("commands", [])
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
