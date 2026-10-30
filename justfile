# The one command surface for this workspace.
#
# Every recipe delegates to Nx — `affected` for the everyday loop, `run-many` for the
# complete sweep — rather than looping over packages by hand. That is not style: with
# seven crates and two SDKs over three toolchains, affected selection is what keeps a
# pull-request cycle usable, and a hand-rolled loop would run all of it every time.
#
# scripts/nx.sh runs the locked install first when a worktree has never had one, so every
# recipe below works from a clean clone.

# bash on every platform, so one script works on all three CI runners.
set shell := ["bash", "-uc"]
set windows-shell := ["bash", "-uc"]

nx := "./scripts/nx.sh"

# List available recipes.
default:
    @just --list

# The cargo-prune-superseded release `pool-maintenance` runs, pinned once: `bootstrap`
# installs exactly this version, and onevcs.toml names that recipe for idle pool slots.
prune_superseded_version := "0.1.1"

# Set up from a clean clone: every toolchain, the judged-lint tier and the pool pruner.
# The pruner is installed everywhere bootstrap runs, CI included; only its pinned-version
# check decides whether an install is needed.
bootstrap:
    @{{nx}} run-many -t bootstrap --all
    @just pool-maintenance-tool

# Install the pinned cargo-prune-superseded, unless that version is the one on PATH.
pool-maintenance-tool:
    @[ "$(cargo-prune-superseded --version 2>/dev/null)" = "cargo-prune-superseded {{prune_superseded_version}}" ] \
        || cargo install --locked --quiet cargo-prune-superseded --version "={{prune_superseded_version}}"

# The maintenance onevcs.toml declares for this repository's idle pool slots: remove each
# unit's superseded builds older than 2 days from every target directory cargo builds
# into here — `target`, the one .cargo/config.toml names — keeping each unit's newest build.
pool-maintenance:
    @cargo-prune-superseded --keep-within 2d target

# The pool-maintenance pass-time budget. Deliberately outside `check` and `gate`: its
# measurement builds the workspace twice. The release pull request runs it after its gate.
# It runs onebudgetspec directly, as the pool-maintenance project's own `budgets-host`
# target does, because a recipe naming an Nx target is read as a stage every gate runs.

# Measure one steady-state pool-maintenance pass against its host-labelled budget.
pool-maintenance-budget:
    @bun run onebudgetspec check pool-maintenance/budgets.yaml --label host

# Fails on any issue: there is no warnings-only mode. The phases are spelled out
# rather than hidden behind one aggregate target so it is visible from here that the
# tests run INSIDE the gate rather than beside it, and so each phase is individually
# invocable. "Affected" means: reachable from the diff against NX_BASE, which defaults
# to nx.json's `defaultBase` (origin/main). CI derives the merge base explicitly and
# exports it; locally, `NX_BASE=<ref> just check` does the same.

# Everyday gate: format, lint, types, tests and coverage over the affected projects.
check: format-check lint typecheck test coverage distribution-check distribution-test

# CI runs this on the default branch; contributors can also run it manually.
# CI derives NX_BASE explicitly. Supply-chain and distribution stages remain unconditional.

# Full quality gate over the affected projects, plus the supply chain. Fails on any issue.
gate: deny check

# scripts/ is a project, so the phases above already reach these: `lint` runs the bash 3.2
# scan and `test` runs the checks that watch it refuse, over the affected projects. This is
# the by-hand entry point the guards' own diagnostics name, and it runs both regardless of
# what the diff touched.

# Prove the scripts still run on the bash 3.2 macos-latest ships.
script-check:
    @{{nx}} run scripts:lint
    @{{nx}} run scripts:test

# Both stages are targets of the scripts project but sit outside its `check` fan-out, so
# they run on every gate rather than by affected selection. What they read is the whole
# publishable surface and several of them clone this repository, which no input glob
# describes — see scripts/project.json.

distribution-check:
    @{{nx}} run scripts:distribution-check

# Prove the Rust cache warmer leaves what a pull request's Linux build reuses: a cold build of
# every dependency three ways into a scratch target directory, twenty to thirty minutes on a
# hosted runner, so no gate runs it. It is a plain script rather than an Nx target because a
# recipe that runs a target by name is a stage every gate has to run
# (scripts/check-live-lane-selection.sh holds that), and this one deliberately is not.
warm-cache-reuse-check:
    @bash scripts/check-warm-rust-cache-reuse.sh

# Its cheap half, over the target directory scripts/warm-rust-cache.sh --instrumented has just
# filled: the warmer's Linux leg runs it, with no Node toolchain for Nx to run on.
warm-cache-reuse-check-warmed:
    @bash scripts/check-warm-rust-cache-reuse.sh --warmed

distribution-test:
    @{{nx}} run scripts:distribution-test

# The five CD jobs of .github/workflows/release.yml, one recipe each: every step of a job is
# one phase of its recipe (`plan`, `build`, `publish --to <registry>`, `verify --from
# <registry>`, `available --from <registry>`, or `run --dry-run` for the whole job with
# nothing sent), so a release runs nothing a contributor cannot. See scripts/release_cd.py.

# The build-linux job: build, package, test-install, publish and verify x86_64 Linux.
release-linux *args:
    @python3 scripts/release_cd.py --platform linux {{args}}

# The build-linux-arm64 job: build, package, test-install, publish and verify arm64 Linux.
release-linux-arm64 *args:
    @python3 scripts/release_cd.py --platform linux-arm64 {{args}}

# The build-macos job: build, package, test-install, publish and verify Apple Silicon.
release-macos *args:
    @python3 scripts/release_cd.py --platform macos {{args}}

# The build-windows job: build, package, test-install, publish and verify x86_64 Windows.
release-windows *args:
    @python3 scripts/release_cd.py --platform windows {{args}}

# The publish job: confirm every platform landed, then publish crates, SDKs and the launcher.
release-publish *args:
    @python3 scripts/release_cd.py --platform independent {{args}}

# The one place the live-lane decision is consulted, so every path that can open a live
# session reaches the API through this recipe. scripts/live-lane-selection.sh prints the
# `--exclude` for each live plugin the diff against NX_BASE cannot reach; an empty or
# failed answer excludes nothing, so the lane runs.

# llmlint: ignore-block[external_service_suite_stays_out_of_the_affected_tier] `affected` is the edge these suites sit behind: a plugin's live tests run only when that plugin's own diff selects it, and the exclusion below is the second half of that edge — a version bump selects every project through Cargo.lock and reaches no plugin behaviour.
# Tests only, for the affected projects.
test:
    @python3 crates/onetaskgraph-github-projects-e2e/lane_runner.py select scripts/live-lane-selection.sh bash {{nx}} affected -t test budgets
# llmlint: ignore-end[external_service_suite_stays_out_of_the_affected_tier]

# Every crate's coverage target runs its tests instrumented into the one shared profile
# directory and keeps its profiles (`--no-report`); the `workspace` project's coverage
# target depends on all of them and enforces the 95% line floor once, over the union. The
# crates' runs depend in turn on workspace:coverage-clear, which empties that directory
# first. Because `workspace` depends on every project, any affected project selects the
# aggregate and so every crate's run — the cost AGENTS.md records for the shared target
# directory. Each SDK still measures itself. The measurement is skipped on Windows with a
# printed notice (see scripts/rust-coverage.sh), where instrumentation does not attribute
# subprocess coverage; the functional lanes still gate that platform.
#
# `cargo llvm-cov` re-runs the very integration tests `test` above just ran, so live
# credentials left set here would open a SECOND session per lane against one shared external
# fixture. scripts/rust-coverage.sh clears them; read the note there before changing this
# recipe or the platform matrix in .github/workflows/ci.yml.

# Coverage only, for the affected projects. Fails below 95% lines over the union of the crates.
coverage:
    @{{nx}} affected -t coverage

# Lint only, for the affected projects.
lint:
    @{{nx}} affected -t lint

# Type check only, for the affected projects.
typecheck:
    @{{nx}} affected -t typecheck

# Check formatting without changing anything, for the affected projects.
format-check:
    @{{nx}} affected -t format-check

# Format every project in place.
format:
    @{{nx}} run-many -t format --all

# Supply-chain gate: banned crates, licences, sources and advisories. Linux-only in CI,
# where it is its own required check.
deny:
    @{{nx}} run workspace:deny

# Linux-only in CI because generated-code drift is platform-independent.
generate-check:
    @task_log="$$(mktemp)"; trap 'rm -f "$$task_log"' EXIT; \
        if ! {{nx}} run sdk-python:generate-check >"$$task_log" 2>&1; then \
            cat "$$task_log" >&2; \
            echo "generation check failed; next: run 'just generate-check' after fixing the reported Python generator error" >&2; \
            exit 1; \
        fi

# Linux CI aggregate: generated-code drift plus the affected-project gate.
check-generated: generate-check check

# Linux CI aggregate: generated-code drift plus the affected-project gate and the supply chain.
gate-generated: generate-check gate

# Upgrade every ecosystem's dependencies, then re-run the complete bar on the result.
upgrade:
    @cargo update --quiet
    @cd sdks/python && uv lock --upgrade --quiet && uv sync --quiet
    @bun update --silent
    @just gate

# Print the JSON Schema bundle both SDKs are generated from.
# llmlint: ignore[tool_output_is_signal] stdout is the schema consumed by SDK generators.
schema:
    @cargo run --quiet -p onetaskgraph --bin onetaskgraph -- schema

# Show the project graph Nx selects against.
graph:
    @{{nx}} graph --file=.nx/graph.html

# Ensures `just`, verifies the rest of the toolchain, then runs setup-llmlint. Runs
# automatically via the Claude Code SessionStart hook; this is the manual entry point.
# Idempotent, no-ops in CI.

# llmlint: ignore[tool_output_is_signal] this recipe is a thin wrapper over
# scripts/session-setup.sh, which carries the same directive and the reason: a
# session-startup installer logs each step and continues rather than blocking startup,
# so a flaky install cannot abort the hook. The output shape is the script's, not this
# recipe's, and quieting it here would hide a provisioning failure from the session.
# Provision the dev toolchain for a session.
session-setup:
    ./scripts/session-setup.sh

# llmlint: ignore[tool_output_is_signal] this recipe is a thin wrapper over
# scripts/setup-llmlint.sh, which carries the same directive and the reason: a
# session-startup installer logs each step and continues rather than blocking startup,
# so a flaky install cannot abort the hook. The output shape is the script's, not this
# recipe's, and quieting it here would hide a provisioning failure from the session.
# Install/refresh the llmlint toolchain (oneharness + llmlint). Idempotent.
setup-llmlint:
    ./scripts/setup-llmlint.sh

# Informational, like a bench: no recipe below is reachable from `check` or `gate`, and
# scripts/check-visual-docs.sh refuses one that becomes so. screenshots/AGENTS.md is why.

# Provision the pinned screenshot renderer.
screenshots-tools:
    @bash scripts/screenshots-freeze.sh ensure

# Capture the terminal screenshots into shots/current/<lane>/ and docs/screenshots/.
screenshots:
    @bash scripts/screenshots.sh

# Recapture and refresh the committed baseline, after an INTENDED output change.
screenshots-bless: screenshots
    @bash scripts/screenshots-bless.sh

# Deliberately OUT of the deterministic gate: it needs an authenticated harness and
# makes network calls. Config is the composed llmlint.yml.

# LLM-judge lint over the configured set (or the paths given).
lint-llm *paths:
    @command -v llmlint >/dev/null 2>&1 || { echo "llmlint not installed — run 'just setup-llmlint'"; exit 1; }
    @llmlint {{paths}}

# Checks config structure, that every `llmlint: ignore` directive names a real rule,
# and that an edited versioned fragment bumped its version. No credential, no model.

# Fast, model-free llmlint gate. CI runs it before spending a harness call.
lint-llm-validate *args:
    @command -v llmlint >/dev/null 2>&1 || { echo "llmlint not installed — run 'just setup-llmlint'"; exit 1; }
    @llmlint validate {{args}}

# This is the blocking `llmlint` pull-request check; run it locally before pushing.

# llmlint scoped to what this branch changed since it forked from the base.
lint-llm-diff base="origin/main" *args:
    @command -v llmlint >/dev/null 2>&1 || { echo "llmlint not installed — run 'just setup-llmlint'"; exit 1; }
    @llmlint --diff --diff-base "{{base}}" {{args}}

# Private workflow tooling stays isolated from the product SDK workspace.
github-workflows-bootstrap:
    @bun install --cwd .github --frozen-lockfile

# Focused workflow graph, required-context and budget proofs.
github-workflows-check:
    @{{nx}} run-many -t check --projects github-workflows

# Explicit scheduled creation or standing-set recovery; mode comes from the caller.
linear-document-creation:
    @{{nx}} run-many -t live-documents --projects onetaskgraph-linear,onetaskgraph-linear-e2e --parallel=1
