import { graphReadSet } from "./workflow-graph.mjs";

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

export const policyJourneyReadSet = [
  "bun.lock",
  "rust-toolchain.toml",
  "scripts/ci-job.sh",
  "scripts/project.json",
  "justfile",
  // Exactly the scripts:distribution-check guards; policy/tests/inventory.test.mjs holds the list to it.
  "scripts/check-distribution-contract.sh",
  "scripts/check-distribution-contract-enforced.sh",
  "scripts/check-release-packaging.sh",
  "scripts/check-release-targets.sh",
  "scripts/check-release-probe.sh",
  "scripts/check-crate-sibling-resolution.sh",
  "scripts/check-crate-sibling-resolution-same-second.sh",
  "scripts/check-store-fixtures.sh",
  "scripts/check-store-fixtures-enforced.sh",
  "scripts/check-release-pr-sync.sh",
  "scripts/check-release-tooling-selection.sh",
  "scripts/check-real-release-preparation.sh",
];

export const policyJourneyFiles = [
  ".github/policy/tests/boundaries.test.mjs",
  ".github/policy/tests/inventory.test.mjs",
  ".github/policy/tests/policy-carriers.test.mjs",
  ".github/policy/tests/policy-routing.test.mjs",
  ".github/policy/tests/policy.test.mjs",
  ".github/policy/tests/release-shortcut.test.mjs",
  ".github/policy/tests/reusable.test.mjs",
  ".github/policy/tests/rust-cache-warm.test.mjs",
];

// The one policy journey run on every platform, because the real action it runs does.
export const rustCacheJourneyFiles = [".github/policy/tests/rust-cache.test.mjs"];

export const cdJourneyFiles = [
  ".github/tests/github-interfaces.test.mjs",
  ".github/tests/release-cd.test.mjs",
  ".github/tests/release-plz.test.mjs",
  ".github/tests/release.test.mjs",
];

// github-workflow-policy is rooted beside .github's own files rather than at .github, so it
// names the .github files it reads as one workspace glob: its own reads, not an outside one.
export const GITHUB_READS = "{workspaceRoot}/.github/**/*";

// What a journey target's inputs get wrong against the read set of the tests it runs: every
// outside read it is not keyed on, and every workspace input its tests do not read.
export function inputProblems(project, target) {
  const expand = (inputs) =>
    inputs.flatMap((input) =>
      typeof input === "string" && project.namedInputs?.[input]
        ? expand(project.namedInputs[input])
        : [input],
    );
  const graph = graphReadSet().filter((path) => !path.startsWith(".github/"));
  if (!graph.length) throw Error("the graph reads no outside file, so this proves nothing");
  const outside = [
    ...new Set([
      ...graph,
      ...(target === "cd-journeys"
        ? cdJourneyReadSet
        : target === "policy-tests"
          ? policyJourneyReadSet
          : []),
    ]),
  ];
  const inputs = expand(project.targets[target].inputs ?? ["default"]);
  const github = target === "policy-tests" ? [GITHUB_READS] : [];
  return [
    ...github
      .filter((entry) => !inputs.includes(entry))
      .map((entry) => `${target} is not keyed on ${entry.slice("{workspaceRoot}/".length)}`),
    ...outside
      .filter((path) => !inputs.includes(`{workspaceRoot}/${path}`))
      .map((path) => `${target} is not keyed on ${path}`),
    ...inputs
      .filter((entry) => String(entry).startsWith("{workspaceRoot}/"))
      .filter((input) => !github.includes(input))
      .filter((input) => !outside.includes(input.slice("{workspaceRoot}/".length)))
      .map((input) => `${target} is keyed on ${input}, which its tests do not read`),
  ];
}
