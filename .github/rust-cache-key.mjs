// The env-var name prefixes the pinned Swatinem/rust-cache hashes into its key besides the
// toolchain and the manifests. rust-cache.test.mjs reconciles this list with that action's
// own source on every run, so it cannot drift from what the action really keys on.
export const HASHED_ENV_PREFIXES = ["CARGO", "CC", "CFLAGS", "CXX", "CMAKE", "RUST"];
