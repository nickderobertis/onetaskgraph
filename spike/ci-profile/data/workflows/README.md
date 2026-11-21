# Historical workflows the measurement-only edits must also apply to

`ci-fa58067f-3956.yml` is `.github/workflows/ci.yml` at fa58067f (#3956, the coverage-caching change) and
`ci-82318e45-parent-of-3956.yml` is the same file at 82318e45, its parent — `git show <commit>:.github/workflows/ci.yml`.
A historical A/B runs each side's own workflow with byte-identical measurement edits, so `test_contracts.py`
applies `measurement_only.py` to both.
