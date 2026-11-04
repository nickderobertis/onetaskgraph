export const fixtureEntryEnvironment = {
  GITHUB_ACTIONS: "",
  RUNNER_TEMP: ".nx/fixture-runner-temp",
  NX_SELF_HOSTED_REMOTE_CACHE_SERVER: "",
  NX_SELF_HOSTED_REMOTE_CACHE_ACCESS_TOKEN: "",
};

// Test subprocesses must not inherit the CI job's Nx recording or cache authority.
// A cache journey opts in explicitly with its own recording tree and sidecar token.
export function fixtureEnvironment(additions = {}) {
  return {
    ...process.env,
    GITHUB_ACTIONS: undefined,
    NX_SELF_HOSTED_REMOTE_CACHE_SERVER: undefined,
    NX_SELF_HOSTED_REMOTE_CACHE_ACCESS_TOKEN: undefined,
    ...additions,
  };
}
