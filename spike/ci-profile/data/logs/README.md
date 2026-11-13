# Lines of real job logs, kept as the profiler's log-format fixtures

`run-43152201170-lines.txt` holds lines copied unchanged (timestamps and terminal colour codes included) from
the logs of run 43152201170's `test-linux` (job 130963785484) and `test-macos` (job 130963785863), read with
`GET repos/nickderobertis/onetaskgraph/actions/jobs/<job>/logs` on 2026-11-13: for each format `jobprofile.py`
reads — Nx task headers executed and replayed, project-prefixed lines, test starts, nextest's libtest-json-plus
events, cargo's `Compiling`/`Finished` lines and file-lock waits, rust-cache's cache size, the cache report's
stored-entries line and the step groups — the first few the logs hold. `test_contracts.py` holds each pattern to them.
