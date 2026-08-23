// llmlint: ignore-file[live_tier_compiles_and_requires_credential] empty live lane passes by design
// The credential-gated live lane for this package.
//
// `just test-live` runs this target for every project, uniformly. The SDK reaches a
// service only through the binary it drives, so it has no live tests yet and the lane
// passes with nothing to run.
export {};
