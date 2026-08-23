# Working in `sdks/`

- **The models are generated from `onetaskgraph schema`, never hand-written.** The binary
  emits the bundle from the same types it serialises, so a generated model cannot drift
  from what it parses — a hand-written one can, and nothing would catch it.
- **Each package states its version once**, and a test compares it against the published
  manifest. The release pipeline writes every manifest through one script; never hand-edit
  one of them.
