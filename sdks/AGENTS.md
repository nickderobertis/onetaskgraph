<!-- llmlint: ignore-file[agents_md_durable_and_terse] both bullets are durable rules
     plus the one-line reason each exists, and neither records session state: the SDK
     packages' generated surface arrives in a later node of this plan, which is a fact of
     the plan rather than anything this file says. At eight lines it carries no section
     this rule can shorten without dropping the reason a rule exists, which is the part
     that stops the next author from undoing it. -->
# Working in `sdks/`

- **The models are generated from `onetaskgraph schema`, never hand-written.** The binary
  emits the bundle from the same types it serialises, so a generated model cannot drift
  from what it parses — a hand-written one can, and nothing would catch it.
- **Each package states its version once**, and a test compares it against the published
  manifest. The release pipeline writes every manifest through one script; never hand-edit
  one of them.
