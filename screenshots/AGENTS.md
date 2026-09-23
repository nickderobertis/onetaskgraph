# Terminal screenshots

Deterministic SVGs of this CLI's **real** output, gated by
[screencomp](https://github.com/nickderobertis/screencomp). Informational, like a bench —
**never part of `just check` or `just gate`**; `.github/workflows/visual-docs.yml` owns the
comparison in CI, and `.githooks/pre-push` owns the local half.

## What it is

`scripts/screenshots.sh` drives the **real release `onetaskgraph` binary** against the
fixture in `fixture/` and renders exactly the bytes it wrote to an SVG with
[`freeze`](https://github.com/charmbracelet/freeze). Nothing is scripted, mocked or
rewritten: each shot is the output of one real invocation.

Two configured sources and nothing else, both reaching nothing:

- `fixture/sync/` — a checkout holding two folders of Markdown (`local-md`): `plans/`,
  where the work is authored, and `work/`, standing in for where a team's tickets live. Two
  of them rather than one so a `copy` is a real copy between real sources rather than a
  staged one.
- `fixture/board/` — the same folder of plans beside an `in-memory` source that declares
  `filter_by_label: unsupported`. `in-memory` is the one plugin whose capability
  declaration a document *chooses*, which is what makes the plan block show both halves of
  the engine's bargain.

**The two plugins that reach a network are never configured here** — not even against the
loopback GraphQL fixture servers the test suite stands up, because a capture that binds a
socket can fail for a reason that has nothing to do with the tool. So the capture needs no
credential, and `sources status-options`, the live lanes and every hosted surface are
deliberately outside it.

## The scenes, and what each one documents

One per surface the README explains, and each sits in the section that explains it:

- **`task-list`** — the hero, immediately under the title: the tool's main usage, five rows
  interleaved across two sources in configured-name order. That interleave *is* the
  product's claim, in the one view a reader meets first.
- **`sources-list`** — beside `## Using it`: each source with the predicates it applies
  itself, how far it walks dependencies, and its largest page. It is what the plan block
  below acts on, so it comes first.
- **`task-show`** — beside the paragraphs on locations and comments: the aligned field
  block, the `location: path …` line, the body, and the one comment the fixture's task
  carries, with that comment's id, author and times. Those paragraphs promise all four.
- **`task-list-explain`** — in `### Seeing which plan you got`, **replacing** a
  hand-written transcript: one `--label` query over two sources of differing capability,
  one correct answer, two plans — `pushed down: label` for the folder, `applied locally:
  label` for the source that declares it cannot. The section taught that in prose over a
  transcript nobody had run; this is the same lesson, run.
- **`task-copy-dry-run`** — under the copy flag table, whose first row is `--dry-run`: one
  item matched to its counterpart and reported `updated`, one with none reported `created`,
  and the line counting rewritten, unresolved and ambiguous references.
- **`task-deps`** — in `## Custom metadata and repositories`, where the prose says an edge
  may cross sources: a same-source edge and one whose far end is in another source, both
  named by kind and qualified id.
- **`config-show`** — in `## Configure`, under the paragraph that says precedence is
  something you can see: every effective setting with the layer it came from, and all four
  kinds of layer in one shot — an environment variable by name, a flag by name, a document
  by path, and a default.

**Two candidate scenes are deliberately absent.** `search` has no section of its own to
document, so a picture of it would have been padding beside a flag list. `onetaskgraph
--help` is the one surface `clap` colourises, but only on a terminal: captured through a
pipe it is plain text, and the README already spells every verb and flag it would show —
in a block a test reconciles against the binary's own help. A shot that says less than the
prose beside it is dropped rather than added.

## Why the fixture is curated rather than real

Curation here is not cosmetic. A real plan root of the kind this stack keeps renders
hex-encoded native ids well over a hundred columns wide, and a `next page:` token is some
two hundred hexadecimal characters on one unwrapped line. Neither photographs, and no
window width fixes it. So the fixture is short ids (`T-1`, `P-1`, `D-1`), short titles,
three labels, both dependency directions including an edge that leaves the source, and one
comment with a fixed timestamp written into the file.

It is small enough to read in one screenful and complete enough that every scene above is a
real query over it rather than a special case.

## Why it is byte-reproducible, and what it is pinned to

screencomp gates on the **hash** of each image, so two captures of one build must be
byte-identical. Unlike a rasterised PNG, whose anti-aliasing drifts across CPUs, an SVG is
pure layout maths. Four things make that hold, and each has exactly one source in this
tree:

| What is pinned | Its one source | What keeps a copy from drifting |
| --- | --- | --- |
| the renderer (`freeze`) | `FREEZE_VERSION` in `scripts/screenshots-freeze.sh` | every caller — the capture, `just screenshots-tools`, the workflow's capture step — reaches the binary *through* that script, and `scripts/check-visual-docs.sh` fails on a freeze version stated anywhere else |
| the font | `fonts/JetBrainsMono-Regular.ttf`, vendored (OFL, see `fonts/JetBrainsMono-OFL.txt`) | named once, in `scripts/screenshots.sh`, which refuses to render without it; `check-visual-docs.sh` fails when either file is gone |
| the arch lane | `[capture].arches` in `screencomp.toml` | the capture, the guard and the bless step each read it from there with the same `sed`, and `check-visual-docs.sh` refuses a second spelling of it |
| screencomp itself | `.github/workflows/visual-docs.yml`, where the `uses:` ref and `screencomp-version:` are the same version | `screencomp doctor --env` reports the pin having parted from the installed CLI, and `check-visual-docs.sh` reconciles the two literals without needing the CLI at all |

The environment is cleared: every `ONETASKGRAPH_*` variable is unset before capture,
because the binary reads those ahead of the configuration document — an exported one prints
straight into the `config show` scene as the layer it won at, and one naming a source adds
that source's rows to every listing. `HOME`, the `XDG_*` directories and `TMPDIR` are all
redirected into the staged tree, so no ambient user configuration and no per-run temporary
path can reach a shot.

**The fixture is staged at a fixed absolute path (`/tmp/otg`), and that is the whole of the
normalisation.** This tool prints absolute paths — a configuration document, a resolved
`root`, a task's `location` — and `config show` aligns its columns against the widest value
it is printing. A path rewritten to a placeholder afterwards would leave two rows' origin
column ninety characters left of every other row's, so the path is made short and identical
up front instead of being edited after the fact. Every byte in every shot is a byte the
binary wrote.

The cost is stated rather than discovered: that path has to *be* what the kernel reports, so
the capture refuses on a platform where it is not — macOS resolves `/tmp` through a symlink
to `/private/tmp`, and the binary canonicalises what it prints. The guard there skips
loudly rather than pushing bytes no baseline can match; CI's Linux container is where the
baseline is made. The other cost: the vendored font is embedded into every SVG as base64,
so each committed image is about 360 KB. That is what makes a shot render identically on
GitHub and crates.io with nothing external to fetch.

## Why a generated capture is not a second statement of the contract

This repository holds its README to the binary's own `--help` with a test, and runs a
judged rule against a second spelling of any contract. A committed screenshot of CLI output
*would* be such a second spelling if it were hand-made — which is exactly what the
transcript in `### Seeing which plan you got` was, and it described a query nobody had run.

It is not one now. Every image is produced by running the real binary, and CI refuses the
capture the moment its bytes diverge from `shots/baseline/<arch>.json`. The baseline **is**
the drift gate, and the binary remains the one source: change what a verb prints and that
scene's SVG changes, and the change is either blessed deliberately or the workflow goes
red.

## Outputs

- `shots/current/<arch>/captures.json` and the SVGs — the capture screencomp reads
  (gitignored; regenerated). `$SHOTS_OUT` overrides the directory; the reusable workflow
  exports it per lane.
- `shots/baseline/<arch>.json` — the committed digest baseline. No images.
- `docs/screenshots/*.svg` — the committed copies the README embeds.

## Commands

- `just screenshots-tools` — provision the pinned `freeze` into its repository-scoped
  location. Needs no Go: it installs the published archive.
- `just screenshots` — capture (builds the release binary, writes the shots and the README
  copies). screencomp is installed separately; see its README.
- `just screenshots-bless` — after an **intended** output change: recapture, then refresh
  `shots/baseline/<arch>.json`. Commit it together with `docs/screenshots/`.
- `screencomp doctor --env` — is the setup actually wired: the guard active, the workflow's
  pin and the installed CLI in step.

## The strict gate, and how its local half is activated

CI (`fail-on-drift: true`) fails when a capture diverges from the committed baseline.

Locally, the guard is `scripts/screenshots-guard.sh`, run from **the pre-push hook this
repository already had** — `.githooks/pre-push`, activated by
`scripts/bootstrap-workspace.sh` setting `core.hooksPath`, which is unchanged. The guard
runs *after* the complete gate that hook already ran, so the bar for a push is what it
always was plus one refusal of its own. It re-captures only when what is being pushed
touches a `[guard].paths` glob, and on drift it refreshes the baseline, builds
`shots/review/index.html` and blocks the push so the new bytes are committed deliberately.

Three properties of it are held by `scripts/check-visual-guard.sh`, which drives the real
guard with screencomp and the capture stubbed at the subprocess seam: it captures on a
relevant push and on no other, it blocks on drift and refreshes both the baseline and the
gallery when it does, and a missing screencomp is a loud skip rather than a silent one —
and never the `onevcs: host-prerequisite:` marker, which belongs to the pinned release-plz
alone.

## Changing the screenshots

Editing what a verb prints — `crates/onetaskgraph/src/render.rs`, the CLI surface, the
engine's plan or copy report, either local plugin — or the fixture, or the scenes in
`scripts/screenshots.sh`, changes these SVGs. That is expected: run
`just screenshots-bless` and commit the new baseline together with `docs/screenshots/`.
Moving the renderer pin or the vendored font reflows every shot; bless once.

Adding a scene means adding its `scene` line, blessing the baseline, and placing the image
in the README section that explains the surface it shows — `scripts/check-visual-docs.sh`
fails on a scene with no baseline entry, a committed image the README embeds nowhere, and
an image the README embeds that is not committed.
