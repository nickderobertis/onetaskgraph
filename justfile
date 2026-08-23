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

# Set up from a clean clone: every toolchain, the git hooks, and the judged-lint tier.
bootstrap:
    @{{nx}} run-many -t bootstrap --all

# Fails on any issue: there is no warnings-only mode, and the tests run inside it
# rather than beside it.

# Everyday gate: format, lint, types, tests and coverage over the affected projects.
check base="origin/main":
    @{{nx}} affected -t check --base="{{base}}"

# This is what .githooks/pre-push runs and what the default branch sweeps on every
# push, so nothing affected-detection could miss goes unchecked.

# Full quality gate over EVERY project, plus the supply chain. Fails on any issue.
gate: deny
    @{{nx}} run-many -t check --all

# Tests only, for the affected projects.
test base="origin/main":
    @{{nx}} affected -t test --base="{{base}}"

# Each project measures its own crate and fails below 95% lines. The measurement is
# skipped on Windows with a printed notice (see scripts/rust-coverage.sh), where
# instrumentation does not attribute subprocess coverage; the functional lanes still
# gate that platform.

# Coverage only, for the affected projects. Fails below 95% lines.
coverage base="origin/main":
    @{{nx}} affected -t coverage --base="{{base}}"

# Lint only, for the affected projects.
lint base="origin/main":
    @{{nx}} affected -t lint --base="{{base}}"

# Type check only, for the affected projects.
typecheck base="origin/main":
    @{{nx}} affected -t typecheck --base="{{base}}"

# Check formatting without changing anything, for the affected projects.
format-check base="origin/main":
    @{{nx}} affected -t format-check --base="{{base}}"

# Format every project in place.
format:
    @{{nx}} run-many -t format --all

# A project with no live tests passes with nothing to run, which is what makes the
# target uniform. Neither hosted plugin's credential is required: without one, that
# plugin's own tests skip rather than fail.

# Sweep the credential-gated live lane across every project.
test-live:
    @{{nx}} run-many -t test-live --all

# Linux-only in CI, where it is its own required check.

# Supply-chain gate: banned crates, licences, sources and advisories.
deny:
    @{{nx}} run workspace:deny

# Upgrade every ecosystem's dependencies, then re-run the complete bar on the result.
upgrade:
    cargo update
    cd sdks/python && uv lock --upgrade && uv sync
    bun update
    @just gate

# Print the JSON Schema bundle both SDKs are generated from.
schema:
    @cargo run --quiet -p onetaskgraph -- schema

# Show the project graph Nx selects against.
graph:
    @{{nx}} graph --file=.nx/graph.html

# Ensures `just`, verifies the rest of the toolchain, then runs setup-llmlint. Runs
# automatically via the Claude Code SessionStart hook; this is the manual entry point.
# Idempotent, no-ops in CI.

# Provision the dev toolchain for a session.
session-setup:
    ./scripts/session-setup.sh

# Install/refresh the llmlint toolchain (oneharness + llmlint). Idempotent.
setup-llmlint:
    ./scripts/setup-llmlint.sh

# Deliberately OUT of the deterministic gate: it needs an authenticated harness and
# makes network calls. Config is the composed llmlint.yml.

# LLM-judge lint over the configured set (or the paths given).
lint-llm *paths:
    @command -v llmlint >/dev/null 2>&1 || { echo "llmlint not installed — run 'just setup-llmlint'"; exit 1; }
    llmlint {{paths}}

# Checks config structure, that every `llmlint: ignore` directive names a real rule,
# and that an edited versioned fragment bumped its version. No credential, no model.

# Fast, model-free llmlint gate. CI runs it before spending a harness call.
lint-llm-validate *args:
    @command -v llmlint >/dev/null 2>&1 || { echo "llmlint not installed — run 'just setup-llmlint'"; exit 1; }
    llmlint validate {{args}}

# This is the blocking `llmlint` pull-request check; run it locally before pushing.

# llmlint scoped to what this branch changed since it forked from the base.
lint-llm-diff base="origin/main" *args:
    llmlint --diff --diff-base "{{base}}" {{args}}
