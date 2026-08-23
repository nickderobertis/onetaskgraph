# AGENTS.md

Durable constraints for humans and agents working here. Terse on purpose — this is
always-loaded context. `just --list` shows the commands; `README.md` explains the product.
`CLAUDE.md` is a symlink to this file; edit only this one.

## What this repo is

One interface over the ticketing systems the user's work lives in (Linear, GitHub
Projects, local Markdown), shipped three ways from one engine: a Rust CLI, a Python SDK,
and a TypeScript SDK.

## Stack and composition

- **Product shape:** `cli`.
- **Language(s):** Rust (the engine, the plugins and the binary), Python (the SDK and the
  maturin-wrapped CLI distribution), TypeScript (the SDK and the npm launcher).
- **References composed (11):** `base.md`, `shapes/cli.md`, `languages/rust.md`,
  `languages/python.md`, `languages/typescript.md`, `intersections/rust-cli.md`,
  `intersections/python-cli.md`, `ci.md`, `llmlint.md`, `releasing.md`, `monorepo.md`.
- **Excluded, and why:** `shapes/web-app.md`, `shapes/react.md`, `shapes/nextjs.md` — this
  ships no web surface; `shapes/asdf-plugin.md` — distribution is crates.io, PyPI and npm,
  not an asdf plugin; `shapes/skills-repo.md` — this is a product, not a skills
  collection. Nothing non-negotiable is excluded.
- **Nx is a deliberate inclusion**, and the reason is the user's own: *affected selection
  is the gate, so a change touching one plugin does not run the other plugins' tests.*
  With seven crates and two SDK packages over three toolchains that is the difference
  between a usable pull-request cycle and an unusable one. Putting Node on the critical
  path of the Rust gate is the accepted cost and is not reopened.
- **`onetaskgraph-plugin-api` exists for that same reason.** With the trait inside the
  engine crate, every plugin would depend on the engine, every engine change would mark
  every plugin affected, and affected selection would buy nothing for the six crates where
  it matters most.

## The plugin contract

It lives in **two** crates, and which one a type is in is the contract, not a detail.

- **`onetaskgraph-plugin-api`** — exactly what a plugin author needs, and nothing else:
  the traits `TaskSource`, `SourcePlugin` and `SecretResolver`; the work types `Task`,
  `Project`, `Label`, `Status`, `StatusCategory`, `DependencyEdge`, `DependencyKind`,
  `Direction`, `NativeId`, `SourceName`; the query and paging types `TaskQuery`,
  `ProjectQuery`, `TextQuery`, `TextFields`, `LabelFilter`, `ProjectFilter`, `PageRequest`,
  `Page`, `Cursor`; the capability types `Capabilities`, `Support`, `DependencySupport`;
  and `SourceError`. **It depends on no other crate of this workspace.**
- **`onetaskgraph-core`** — the engine, plus the reporting types `QueryResponse`,
  `QueryPlan`, `SourcePlan`, `Predicate`, `PageToken`, `SourceFailure` and `GlobalId`.

A type needed on both sides goes in the api crate, because the dependency only runs one
way. `NativeId`, `SourceName`, `Page`, `PageRequest`, `Cursor` and `SourceError` are there
although the engine handles all six: each appears in a trait signature. `GlobalId` is
**not**, deliberately — a plugin never sees a qualified id.

**Keep the api crate still.** Every change to it rebuilds and re-tests every plugin. When a
new type could sit on either side, put it in `onetaskgraph-core` unless a trait signature
names it.

> **Open contract question — `Health`.** The approved enumeration of the api crate's
> contents is exhaustive and does not name `Health`, but `TaskSource::health` returns it
> and that trait is in the api crate, so the enumeration and the trait as written cannot
> both stand. It sits in the api crate today because that is the only placement that
> compiles; see the note on the type. Resolving it — add it to the enumeration, or
> redesign `health` so no such type crosses the boundary — belongs to the contract's owner.
> Do not read the enumeration's silence as an answer.

### The three capability rules

Every implementation of `TaskSource` is bound by all three, and the engine's compensation
is only correct while they hold:

1. A plugin **applies** every predicate it declares `Native`.
2. A plugin **ignores** every `Support`-typed predicate it declares `Unsupported` — it
   returns the *wider* result set, never a narrower one. Silently dropping rows for a
   predicate it did not declare is the one failure no test above the plugin can catch.
   This reaches the `Support`-typed predicates alone. A corollary the in-memory source
   proves: a predicate a source can only *half* apply (a `title-or-content` search where
   only titles are native) must be ignored outright, because half-applying it narrows.
3. The engine compensates transiently — at most one source page plus the caller's page,
   nothing written down. For a `ForwardOnly` source it answers `DependedOnBy` by a bounded
   page-by-page scan and reports it as emulated. A dependency read is never ignored and
   never silently empty.

`DependencySupport` has no unsupported variant on purpose: dependency traversal is a
guaranteed capability of this product, not one a source may opt out of. `Support` — which
governs the filters and the searches — is a different enum and keeps its `Unsupported`
variant, because in-memory compensation for those is sound. Do not conflate the two.

## Invariant: no work data outside a plugin

Nothing of a user's work is stored, cached, indexed or mirrored outside the plugin that
owns it. Three mechanisms enforce that; extend them rather than rediscovering the rule.

1. **Banned dependencies** — `deny.toml` refuses `sled`, `rusqlite`, `libsqlite3-sys`,
   `redb`, `sqlx`, `tantivy`, `cacache`, `moka`, `cached` and `lru` anywhere in the graph.
   `deny` is a required check, so a change that reaches for one cannot merge. **In place.**
2. **The sentinel journey** — `crates/onetaskgraph/tests/e2e/no_persistence.rs` redirects
   `HOME` and every `XDG_*` and `TMPDIR` into one sandbox, plants unique sentinels in a
   `local-md` source, drives every query verb, then fails naming the path if any file
   created during the run holds a sentinel or appears outside the source's own directory.
   It asserts on the observable effect, so it catches caching by any technique.
   **Owed by the `local-md` node.**
3. **The re-ask assertion** — `crates/onetaskgraph-core/tests/no_reuse.rs`, the in-process
   half a filesystem scan cannot see: the in-memory source counts trait calls, and running
   one query twice through one engine instance must ask the source twice.
   **Owed by the engine node.**

## Invariant: no plugin crate depends on the engine

Not as a dependency, not as a dev-dependency, at no depth. Prose does not hold this — the
first worker who wants one helper out of `onetaskgraph-core` will reach for it — so it is
mechanical, by two mechanisms that fail at different moments:

1. **`deny.toml`** permits `onetaskgraph-core` exactly one wrapper, the binary. A plugin
   that adds the engine fails the `deny` job, which is a required check.
2. **`scripts/check-plugin-isolation.sh`**, inside `just check`, reads the real graph from
   `cargo metadata` — never a hand-maintained list — and fails naming the crate and the
   edge, in seconds locally rather than minutes later in CI. It fails the same way if
   `onetaskgraph-plugin-api` gains a dependency on another crate of this workspace.

## The three selections the project graph owes

The split buys exactly one thing and it is a build-graph thing, so the graph is not correct
until it *selects* this way. `scripts/check-affected-selection.sh` makes each edit in a
scratch clone, commits it, and asserts on what real Nx returns — reading the config and
reasoning about it does not count.

1. Editing `onetaskgraph-plugin-api` selects **every** plugin crate.
2. Editing `onetaskgraph-core` outside it selects **no** plugin crate. This is the return
   on the split and the one that fails silently: an over-broad `implicitDependencies`
   entry, or a `namedInputs` glob reaching past its own crate, makes every engine commit
   run every plugin's tests and nothing complains.
3. Editing one plugin selects that crate and its dependents — never a sibling plugin.

Nx cannot read a Cargo manifest, so each Rust `project.json` mirrors its crate's Cargo
dependencies as `implicitDependencies`, and `scripts/check-nx-graph.sh` (inside `just
check`) fails naming the pair when the two disagree **in either direction** — a missing
edge under-runs the gate, an extra one over-runs it.

## Tests

The suite is the only QA loop; realism and completeness are rules, not preferences.

- **Never mock the layer under test.** Every journey drives the compiled binary as a
  subprocess and asserts on exit code, stdout and stderr.
- **Coverage: 95% lines, per project, and each project measures only its own crate.**
  A workspace average lets a weak crate hide behind a strong one — and, decisively, a
  workspace-wide pass runs every crate's tests on every change, which is what affected
  selection exists to avoid. The *measurement* is skipped on Windows with a printed
  notice (instrumentation there does not attribute subprocess coverage); the functional
  lanes still gate that platform.
- **The live lane** is a uniform `test-live` target on every project, empty ones included.
  It is **not** a required check, and that is a decision: a required check a third party
  can turn red is a check that stops being trusted, and a Linear or GitHub outage must not
  block an unrelated merge.

### The journeys this repository owes

Each drives the real binary as a subprocess, and each runs against **every** configured
source kind through one shared fixture table — so a plugin is never proven by a suite of
its own writing. The list grows as features land. ✔ = landed.

1. List tasks from one source.
2. List tasks from several named sources at once, and from every configured source.
3. Show one task by its qualified id.
4. List projects; show one project.
5. A task belonging to no project is listed by default and selected on its own.
6. List labels across sources.
7. Filter tasks by one label, by several at once, and by exclusion.
8. Filter tasks by status category.
9. Search by title only, by content only, and by either — over tasks and projects.
10. Task dependencies forward and reverse — reverse against a `BothDirections` source, and
    again against a `ForwardOnly` one where the bounded scan answers and the plan says so.
11. Project dependencies, forward and reverse, both ways again.
12. A source declaring a predicate unsupported still returns the correct rows, and the plan
    names that predicate as applied locally.
13. A source declaring a predicate native has it pushed down and the plan says so — one
    query against two sources of differing capability, one correct answer, two plans.
14. One source failing leaves the others' results intact, names the failure, and exits
    non-zero unless partial results were allowed.
15. Paging: a limit smaller than the result set walks to exhaustion in a stable order.
16. Configuration precedence: file; environment over file; flag over environment; and the
    effective-configuration verb naming which layer each setting came from.
17. A field of one **named source** set at each of those three layers in turn.
18. The secrets file supplies a variable a config names, and does not override one the
    process environment already defines.
19. Every journey above, again, through a subprocess-wrapped source.
20. The no-persistence sentinel journey.
21. Machine-readable output validates against the schema the binary itself emits.
22. Failure and recovery: unknown source name, malformed configuration, unknown id, and an
    unreachable source each exit non-zero with the problem and a suggested next action on
    stderr.

✔ `--help`, `--version` and `schema` (the bundle both SDKs are generated from), plus their
failure paths — unknown verb, unknown flag, no verb, a closed stdout.

## Recorded decisions

- **The SDKs drive the real binary as a subprocess.** They do not reimplement the engine
  and do not link it. One implementation of the query semantics means the CLI, a script
  and a TypeScript application cannot answer the same question differently — which is the
  whole reason this repository exists. The cost is a process spawn per call.
- **The stdio plugin protocol is the named upgrade path.** If a longer-lived session is
  ever wanted — to amortise the spawn, or to hold a connection open — the answer is the
  JSON-over-stdio seam the contract is already shaped for (`SourceError` carries owned data
  only for exactly this reason), not an in-process binding that would fork the engine.

## Commits, releases, and merging

- **Squash-merge only, via pull request, with auto-merge.** Merge commits and rebase
  merging are off, so one PR is one commit whose subject is the PR title and whose body is
  the PR description. Queue with `gh pr merge --auto --squash`. Head branches auto-delete.
- **Six required checks, and they are the only thing that can refuse a merge here:**
  `check (ubuntu-latest)`, `check (macos-latest)`, `check (windows-latest)`, `deny`,
  `llmlint`, `pr-title` — with `strict: true` (a branch must be up to date with `main`),
  `required_approving_review_count: 0`, linear history, conversation resolution, no
  force-push and no branch deletion, and fork-PR workflow approval for all external
  contributors. Admins may break the glass. Re-apply or verify with the create-repo
  skill's `setup_github_governance.py`.
  **Zero approvals is load-bearing:** nobody reviews these pull requests, and any non-zero
  value would make auto-merge wait forever for a review that never comes.
  Adding a required check costs a change in the orchestration repository, whose inventory
  of this repository's merge path is checked against GitHub on every one of its gate runs.
- **Bump policy (pre-1.0):** `feat` → minor; `feat!` / `BREAKING CHANGE` → minor (a
  breaking change before 1.0 is *not* a major); `fix` / `perf` / `refactor` / `build` →
  patch; `chore` / `docs` / `ci` / `test` / `style` → no release. Post-1.0 a breaking
  change becomes a major. The PR title is what the release tool parses, which is why
  `pr-title` is a required check.
- **Release driver: `release-plz`,** in its release-PR shape — the bot accumulates
  unreleased commits into a PR and merging it cuts the release; the tag then fires the
  build and publish workflows. It must authenticate with `RELEASE_PLZ_TOKEN` (declared in
  `gh-secrets.json`), because a tag pushed by the default `GITHUB_TOKEN` fires no
  downstream workflow and the release would silently ship nothing.
  **The pipeline itself lands with the distribution node; only the decision is recorded
  here.**

## Conventions

- **Scripts are context.** Quiet on success; on failure print the exact problem and a
  concrete next action.
- **Suppress narrowly.** A diagnostic is an error or a suppression at that one site with a
  stated reason. `notignored` posts every suppression a PR adds, so they are read.
- **`gh-secrets.json` is tracked and load-bearing.** It declares the repository secrets
  this build needs and its only destination is the GitHub repository — it writes no local
  `.env` and is unrelated to `~/.config/onetaskgraph/secrets.env`, which is hand-managed.
  Its `RELEASE_PLZ_TOKEN` is sourced from the Bitwarden item named `GH_TOKEN` on purpose.
  Do not rewrite, regenerate or reformat it.
- **Keep the allowlist current.** New routine commands belong in `.claude/settings.json`
  rather than being re-approved every session.
