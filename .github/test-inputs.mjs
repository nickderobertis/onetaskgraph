// Repository files read by each journey target, beyond the workflow graph.
// Globs include the source payloads cargo, maturin and npm build/package, not just their
// manifests. ci-contexts.spec reconciles this inventory with the target that runs them;
// graph-only ci-contexts retains workflow-graph.graphReadSet() alone.
export const cdJourneyReadSet = [
  "config/github-interfaces.toml",
  "pyproject.toml",
  "release-plz.toml",
  "npm/**/*",
  "scripts/release_cd.py",
  "scripts/cli_wheel_lock.py",
  "scripts/test-distribution.sh",
  "scripts/file-lock.sh",
  "scripts/verify-release-products.py",
  "scripts/release-wheel-binary.py",
  "scripts/loopback-release-registries.py",
  "scripts/enable-release-automerge.py",
  "scripts/publish-npm.sh",
  "scripts/npm-registry-auth.sh",
  "scripts/install.sh",
  "scripts/crate-publication-status.sh",
  "scripts/crate-sibling-resolution.sh",
  "scripts/prepare-release-pr.sh",
  "scripts/select-release-version.sh",
  "scripts/scoped-release-plz.sh",
  "scripts/set-version.sh",
  "scripts/scratch-clone.sh",
  "justfile",
  "Cargo.toml",
  "Cargo.lock",
  "rust-toolchain.toml",
  ".cargo/**/*",
  "crates/**/*",
  "sdks/**/*",
  "README.md",
  "LICENSE",
  "uv.lock",
  "bun.lock",
  "package.json",
];

export const policyJourneyReadSet = ["bun.lock", "rust-toolchain.toml", "scripts/ci-job.sh"];

export const policyJourneyFiles = [
  ".github/policy/tests/boundaries.test.mjs",
  ".github/policy/tests/policy-carriers.test.mjs",
  ".github/policy/tests/policy-routing.test.mjs",
  ".github/policy/tests/policy.test.mjs",
  ".github/policy/tests/release-shortcut.test.mjs",
  ".github/policy/tests/reusable.test.mjs",
  ".github/policy/tests/rust-cache-warm.test.mjs",
  ".github/policy/tests/rust-cache.test.mjs",
];

export const cdJourneyFiles = [
  ".github/tests/github-interfaces.test.mjs",
  ".github/tests/release-cd.test.mjs",
  ".github/tests/release-plz.test.mjs",
  ".github/tests/release.test.mjs",
];
