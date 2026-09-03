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
check: format-check lint typecheck test coverage distribution-check distribution-test

# This is what .githooks/pre-push runs and what the default branch sweeps on every
# push, so nothing affected-detection could miss goes unchecked.

# Full quality gate over EVERY project, plus the supply chain. Fails on any issue.
gate: deny distribution-check distribution-test
    @{{nx}} run-many -t check --all

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

distribution-test:
    @{{nx}} run scripts:distribution-test

# llmlint: ignore-block[external_service_suite_stays_out_of_the_affected_tier] `affected` is the edge these suites sit behind; AGENTS.md records why they have no other.
# Tests only, for the affected projects.
test:
    @{{nx}} affected -t test
# llmlint: ignore-end[external_service_suite_stays_out_of_the_affected_tier]

# Each project measures its own crate and fails below 95% lines. The measurement is
# skipped on Windows with a printed notice (see scripts/rust-coverage.sh), where
# instrumentation does not attribute subprocess coverage; the functional lanes still
# gate that platform.
#
# `cargo llvm-cov` re-runs the very integration tests `test` above just ran, so live
# credentials left set here would open a SECOND session per lane against one shared external
# fixture. scripts/rust-coverage.sh clears them; read the note there before changing this
# recipe or the platform matrix in .github/workflows/ci.yml.

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
