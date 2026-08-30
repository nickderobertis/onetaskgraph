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

# Fails on any issue: there is no warnings-only mode. The phases are spelled out
# rather than hidden behind one aggregate target so it is visible from here that the
# tests run INSIDE the gate rather than beside it, and so each phase is individually
# invocable. "Affected" means: reachable from the diff against NX_BASE, which defaults
# to nx.json's `defaultBase` (origin/main). CI derives the merge base explicitly and
# exports it; locally, `NX_BASE=<ref> just check` does the same.

# Everyday gate: format, lint, types, tests and coverage over the affected projects.
check: script-check format-check lint typecheck test coverage distribution-check distribution-test

# This is what .githooks/pre-push runs and what the default branch sweeps on every
# push, so nothing affected-detection could miss goes unchecked.

# Full quality gate over EVERY project, plus the supply chain. Fails on any issue.
gate: script-check deny distribution-check distribution-test
    @{{nx}} run-many -t check --all

# Nx maps no project to scripts/, so `nx affected` selects nothing at all for a change that
# only edits one — exactly the change these checks exist to catch. Hence a recipe rather than
# an Nx target.

# Prove the scripts still run on the bash 3.2 macos-latest ships. Seconds, so it goes first.
script-check:
    @scripts/check-bash4-array-builtins.sh
    @scripts/check-bash4-array-builtins-enforced.sh
    @scripts/check-guard-path-spelling.sh
    @scripts/check-line-reads.sh

distribution-check:
    @task_log="$$(mktemp)" || { echo "distribution check could not create its log; next: inspect temporary-directory permissions and free space" >&2; exit 1; }; trap 'rm -f "$$task_log"' EXIT; \
        if ! scripts/set-version.sh --check >"$$task_log" 2>&1; then \
            cat "$$task_log" >&2; \
            echo "version manifests disagree; next: run 'scripts/set-version.sh <VERSION>' and commit every changed manifest and lockfile" >&2; \
            exit 1; \
        fi
    @scripts/check-distribution-contract.sh
    @scripts/check-distribution-contract-enforced.sh
    @scripts/check-store-fixtures.sh
    @scripts/check-store-fixtures-enforced.sh
    @scripts/check-release-pr-sync.sh
    @scripts/check-release-tooling-selection.sh
    @scripts/check-real-release-preparation.sh

distribution-test:
    @task_log="$$(mktemp)" || { echo "distribution journey could not create its log; next: inspect temporary-directory permissions and free space" >&2; exit 1; }; trap 'rm -f "$$task_log"' EXIT; \
        if ! scripts/test-distribution.sh >"$$task_log" 2>&1; then \
            cat "$$task_log" >&2; \
            echo "distribution journey failed; next: run 'scripts/test-distribution.sh' and fix the named installer or launcher assertion" >&2; \
            exit 1; \
        fi
    @scripts/check-npm-publish.sh
    @scripts/check-fixture-discrimination.sh

# Tests only, for the affected projects.
test:
    @{{nx}} affected -t test

# Each project measures its own crate and fails below 95% lines. The measurement is
# skipped on Windows with a printed notice (see scripts/rust-coverage.sh), where
# instrumentation does not attribute subprocess coverage; the functional lanes still
# gate that platform.

# Coverage only, for the affected projects. Fails below 95% lines.
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

# A project with no live tests passes with nothing to run, which is what makes the
# target uniform. Neither hosted plugin's credential is required: without one, that
# plugin's own tests skip rather than fail.

# Sweep the credential-gated live lane. Pass a project to run just that one — which is
# what .github/workflows/live.yml does, so each hosted plugin's job stays scoped to its
# own crate and its own single credential.
test-live projects="":
    @scripts/test-live.sh {{projects}}

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

# Linux CI aggregate: generated-code drift plus the all-project gate.
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
