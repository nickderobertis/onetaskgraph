# Working in `sdks/`

Subtree rules. The decision behind them is in the root `AGENTS.md`.

- **The models are generated from `onetaskgraph schema`, never hand-written.** The binary
  emits the bundle from the same types it serialises, so a generated model cannot drift
  from what it parses — a hand-written one can, and nothing would catch it.
- **An SDK drives the installed binary as a subprocess.** It does not link the engine and
  does not reimplement the query semantics.
- **Each package states its version once**, and a test compares it against the published
  manifest. The release pipeline writes every manifest through one script; never hand-edit
  one of them.
- **`test-live` is a uniform target here too.** An empty live lane passes with nothing to
  run; when live tests land they skip without a credential rather than failing.
