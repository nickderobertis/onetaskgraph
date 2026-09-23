<!-- llmlint: ignore-file[agents_md_durable_and_terse] this file is in the position the
     root `AGENTS.md` records for itself, and for the same reason: the sections this rule
     reads as non-terse are required content rather than an authoring choice. The scene
     inventory, the account of why the fixture is curated, the pin table naming the one
     source of each restated version, the capture and re-bless commands, and the argument
     that a hash-gated capture is a rendering of the surface rather than a second statement
     of it are each demanded by the acceptance criteria this adoption was written against —
     the pin table by the one that requires the tree to NAME, for every version the capture
     restates, which source or which check holds the copies to it. Two passes of trimming
     removed what was genuinely restated (the guard's case inventory, the outputs
     walkthrough, a duplicated paragraph); what is left is the content those criteria ask
     for, and a third pass would delete it rather than tighten it. Tightening the wording
     is available; removing the content is not. -->

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
pure layout maths. Four things are pinned, and each has exactly one source:

| What is pinned | Its one source | What keeps a copy from drifting |
| --- | --- | --- |
| the renderer (`freeze`) | `FREEZE_VERSION` in `scripts/screenshots-freeze.sh` | every caller reaches the binary *through* that script, and `scripts/check-visual-docs.sh` fails on a freeze version stated anywhere else |
| the font | `fonts/JetBrainsMono-Regular.ttf`, vendored (OFL, see `fonts/JetBrainsMono-OFL.txt`) | the capture refuses to render without it, and `check-visual-docs.sh` fails when either file is gone |
| the arch lane | `[capture].arches` in `screencomp.toml` | the capture, the guard and the bless step each read it from there, and `check-visual-docs.sh` refuses a second spelling |
| screencomp itself | the `uses:` ref and `screencomp-version:` in `.github/workflows/visual-docs.yml` | `screencomp doctor --env` reports the pin having parted from the installed CLI, and `check-visual-docs.sh` reconciles the two literals without needing the CLI |

**Every `ONETASKGRAPH_*` variable is cleared before capture**, because the binary reads
those ahead of the configuration document: an exported one prints straight into the
`config show` scene as the layer it won at, and one naming a source adds that source's rows
to every listing. `HOME`, the `XDG_*` directories and `TMPDIR` are redirected into the
staged tree for the same reason.

**The fixture is staged at a fixed absolute path, and that is the whole of the
normalisation.** This tool prints absolute paths, and `config show` aligns its columns
against the widest value it is printing — so a path rewritten to a placeholder afterwards
would leave two rows' origin column ninety characters left of every other row's. The path
is made short and identical up front instead, and every byte in every shot is a byte the
binary wrote.

Two costs, stated rather than discovered. That path has to *be* what the kernel reports, so
the capture refuses where it is not — macOS resolves `/tmp` through a symlink, and the
binary canonicalises what it prints; the guard there skips loudly and CI's Linux container
is where the baseline is made. And the vendored font is embedded into every SVG as base64,
so each committed image is about 360 KB — which is what makes a shot render identically on
GitHub and crates.io with nothing external to fetch.

## Why a generated capture is not a second statement of the contract

A committed screenshot of CLI output *would* be a second spelling of this tool's surface if
it were hand-made — which is what the transcript in `### Seeing which plan you got` was, and
it described a query nobody had run. This is not: every image is produced by running the
real binary, and CI refuses the capture the moment its bytes leave the committed baseline.
The baseline **is** the drift gate and the binary remains the one source.

## Where this machinery lives

This directory holds the *data* — the curated fixture, the vendored font, and this note —
and `screenshots/project.json` makes it a project like any other. Every **script** of it
lives under `scripts/`, with every other shell script of this repository, and that is
enforcement rather than habit: three commands of the `scripts` project enumerate that one
directory. `check-bash4-array-builtins.sh` scans it for the two bash 4 array builtins
macOS's bash 3.2 does not have, `check-guard-path-spelling.sh` copies the whole of it into
a scratch clone and drives the guards there, and `check-relative-interpreter.sh` names its
subjects by that path. A capture script filed here instead would escape all three in
silence — which is a portability defect that surfaces on the Windows or macOS lane as
whatever it was proving having gone wrong.

So `screenshots:lint` and `screenshots:test` invoke `scripts/check-visual-*.sh` by path,
which is the same shape `workspace` has to every `scripts/check-*.sh` it runs. Each of the
six scripts carries a one-line pointer back to this section rather than restating it.

## Commands, and what is committed

`just screenshots` captures. `just screenshots-bless` recaptures and refreshes the
baseline after an **intended** change, which is then committed together with
`docs/screenshots/`. `just screenshots-tools` provisions the pinned renderer, and
`screencomp doctor --env` says whether the setup is wired at all.

Committed: the digest baseline and the images the README embeds. Regenerated and ignored:
every other tree under `shots/`.

## The strict gate, and how its local half is activated

CI fails on a capture that has drifted. Locally the same comparison runs from **the
pre-push hook this repository already had** — activated by
`scripts/bootstrap-workspace.sh` setting `core.hooksPath`, which is unchanged — and
*after* the complete gate that hook already ran, so the bar for a push is what it always
was plus one refusal of its own. Drift refreshes the baseline and blocks the push, which
is what makes new bytes a deliberate commit rather than a red workflow after the fact.

A missing screencomp is a loud skip and never the `onevcs: host-prerequisite:` marker:
that one belongs to the pinned release-plz, because it tells the engine driving this host
to stop retrying, and a missing renderer is not that.
`scripts/check-visual-guard.sh` is what keeps any of this from quietly stopping.

## Changing the screenshots

Editing what a verb prints, the fixture, or the scenes in `scripts/screenshots.sh` changes
these SVGs. That is expected: `just screenshots-bless`, then commit the new baseline with
`docs/screenshots/`. Moving the renderer pin or the font reflows every shot; bless once.
Adding a scene means its `scene` line, a blessed baseline, and the image placed in the
README section that explains the surface it shows; `check-visual-docs.sh` refuses any two of
those three without the rest.
