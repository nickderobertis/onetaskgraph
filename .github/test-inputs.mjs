import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { canonicalInputNames } from "./nx-policy/version-inputs.mjs";
import { graphReadSet } from "./workflow-graph.mjs";

// Repository files read by each journey target, beyond the workflow graph.
// Globs include the source payloads cargo, maturin and npm build/package, not just their
// manifests. ci-contexts.spec reconciles this inventory with the target that runs them;
// graph-only ci-contexts retains workflow-graph.graphReadSet() alone.
export const cdJourneyReadSet = [
  ".tool-versions",
  "scripts/ci-tool-pins.sh",
  ".github/test-setup.mjs",
  ".github/fixture-environment.mjs",
  ".github/release-published-paths",
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
  // rust-cache-warm.test.mjs runs the copied prune and headroom over a recorded listing.
  "budgets.yaml",
  "scripts/actions-cache-prune.mjs",
  "scripts/actions-cache-prune.versions",
  "scripts/actions-budget-figures.sh",
  "scripts/actions-budget-figures.versions",
  "scripts/nx-remote-cache.mjs",
  "scripts/nx-remote-cache.versions",
  // Exactly the scripts:distribution-check and distribution-test guards; policy/tests/inventory.test.mjs holds the list to it.
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
  "scripts/check-npm-publish.sh",
  "scripts/check-fixture-discrimination.sh",
];

export const policyJourneyFiles = [
  ".github/policy/tests/boundaries.test.mjs",
  ".github/policy/tests/inventory.test.mjs",
  ".github/policy/tests/policy-routing.test.mjs",
  ".github/policy/tests/semantic-release.test.mjs",
  ".github/policy/tests/release-shortcut.test.mjs",
  ".github/policy/tests/reusable.test.mjs",
  ".github/policy/tests/rust-cache-warm.test.mjs",
];

// Repository files the budget journeys read, beyond the workflow graph: the root budgets file,
// the gate's recipe, wrapper and reporter, and the onebudgetspec pin they run (package.json and
// its lockfile). ci-events also reads this repository's own git history, which no Nx input can
// name: the release commit and the push it pins are immutable.
export const budgetJourneyReadSet = [
  "budgets.yaml",
  ".github/reporting/action-v0.1.2.yml",
  ".github/governance.json",
  "justfile",
  "package.json",
  "bun.lock",
  "scripts/gate-time.sh",
  "scripts/gate-time-budget.mjs",
];

export const budgetJourneyFiles = [
  ".github/tests/ci-events.test.mjs",
  ".github/tests/gate-time.test.mjs",
];

// Every file the release tail reads outside its own tests and fixtures: release.yml, whose
// install step is also onevcs's pin, rust-cache-warm.yml, whose workflow name the CD budgets
// exclude, the YAML parser's lock, the root budgets file, the copied figures script and
// onebudgetspec's pin, which that step must match. It imports nothing of the
// run graph. A target of github-workflow-policy's, which no crate edit selects, because it
// installs both tools from npm.
export const releaseTailReadSet = [
  ".github/test-setup.mjs",
  ".github/fixture-environment.mjs",
  ".github/workflows/release.yml",
  ".github/workflows/rust-cache-warm.yml",
  ".github/package.json",
  ".github/bun.lock",
  "budgets.yaml",
  "package.json",
  "scripts/actions-budget-figures.sh",
  ".github/release-published-paths",
];

export const releaseTailFiles = [".github/policy/tests/release-tail.test.mjs"];

// The one policy journey run on every platform, because the real action it runs does.
export const rustCacheJourneyFiles = [".github/policy/tests/rust-cache.test.mjs"];

export const inputValidationReadSet = [
  "nx.json",
  "scripts/nx-version-free-inputs.mjs",
  ".github/nx-policy/version-inputs.mjs",
];

export const nxHashFiles = [".github/nx-policy/tests/nx-hashes.test.mjs"];

export const nxInputFiles = [".github/nx-policy/tests/nx-inputs.test.mjs"];
export const nxInputReadSet = [
  ".tool-versions",
  "justfile",
  "scripts/ci-tool-pins.sh",
  "scripts/scoped-release-plz.sh",
  ".github/test-setup.mjs",
  ".github/fixture-environment.mjs",
  ".github/nx-policy/tests/nx-inputs.test.mjs",
  ".github/nx-policy/tests/fixtures/nx-workspace-contracts.mjs",
  "nx.json",
  "**/project.json",
  "package.json",
  "bun.lock",
  "scripts/ci-job.sh",
];

export const cacheJourneyFiles = [
  ".github/cache-policy/tests/cache.test.mjs",
  ".github/cache-policy/tests/fixture-isolation.test.mjs",
  ".github/cache-policy/tests/fixture-entry.test.mjs",
];

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
  const rootDefinition = JSON.parse(readFileSync(new URL("../nx.json", import.meta.url), "utf8"));
  const rootInputs = rootDefinition.namedInputs;
  assert.ok(
    rootInputs &&
      typeof rootInputs === "object" &&
      !Array.isArray(rootInputs) &&
      Object.values(rootInputs).every(Array.isArray),
    "Nx named inputs are arrays",
  );
  const expand = (inputs) =>
    inputs.flatMap((input) =>
      typeof input === "string" && (project.namedInputs?.[input] ?? rootInputs[input])
        ? expand(project.namedInputs?.[input] ?? rootInputs[input])
        : [input],
    );
  // The release tail imports nothing of the run graph, so it is keyed on its own reads alone.
  const graph = ["release-tail", "nx-inputs"].includes(target)
    ? []
    : [...graphReadSet().filter((path) => !path.startsWith(".github/")), ...inputValidationReadSet];
  if (!graph.length && !["release-tail", "nx-inputs"].includes(target))
    throw Error("the graph reads no outside file, so this proves nothing");
  const outside = [
    ...new Set([
      // Execution tools are inputs as well as the data each test reads.
      ...((project.targets[target].cache ?? rootDefinition.targetDefaults[target]?.cache) === true
        ? [".tool-versions"]
        : []),
      ...graph,
      ...(target === "cd-journeys"
        ? cdJourneyReadSet
        : target === "policy-tests"
          ? policyJourneyReadSet
          : target === "budget-journeys"
            ? budgetJourneyReadSet
            : target === "nx-inputs"
              ? nxInputReadSet
              : target === "release-tail"
                ? releaseTailReadSet
                : []),
    ]),
  ];
  const inputs = expand(project.targets[target].inputs ?? ["default"]);
  const semanticNames = canonicalInputNames();
  const semantic = inputs.flatMap((input) => {
    const ecosystem =
      typeof input?.runtime === "string"
        ? input.runtime.match(
            /^node scripts\/nx-version-free-inputs\.mjs --ecosystem (cargo|npm|python)(?: --keep-version)?$/,
          )?.[1]
        : undefined;
    return ecosystem ? semanticNames[ecosystem] : [];
  });
  const covered = (path) =>
    inputs.includes(`{workspaceRoot}/${path}`) || semantic.includes(path.split("/").at(-1));
  const github = target === "policy-tests" ? [GITHUB_READS] : [];
  return [
    ...github
      .filter((entry) => !inputs.includes(entry))
      .map((entry) => `${target} is not keyed on ${entry.slice("{workspaceRoot}/".length)}`),
    ...outside.filter((path) => !covered(path)).map((path) => `${target} is not keyed on ${path}`),
    ...inputs
      .filter((entry) => String(entry).startsWith("{workspaceRoot}/"))
      .filter((input) => !github.includes(input))
      .filter((input) => !outside.includes(input.slice("{workspaceRoot}/".length)))
      .map((input) => `${target} is keyed on ${input}, which its tests do not read`),
  ];
}
