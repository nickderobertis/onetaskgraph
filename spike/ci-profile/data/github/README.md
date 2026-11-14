# GitHub's own answers, kept as the harness's contract fixtures

Each file is a response GitHub's REST API gave this spike's own reads, kept as GitHub returned it, so
`test_contracts.py` holds `models.py`'s GitHub records to the producer instead of to hand-written stand-ins.

| File | Read | When |
| --- | --- | --- |
| `run-43152201170-jobs.json` | `GET repos/nickderobertis/onetaskgraph/actions/runs/43152201170/jobs?per_page=50` | 2026-11-13 |
| `run-43369924587-artifacts.json` | `GET repos/nickderobertis/onetaskgraph/actions/runs/43369924587/artifacts?per_page=100` | 2026-11-13 |
| `runs-listing-page.json` | `GET repos/nickderobertis/onetaskgraph/actions/workflows/ci.yml/runs?event=pull_request&created=2026-11-09T23:15:00Z..2026-11-13T23:59:00Z&per_page=100&page=1`, its first three runs kept whole | 2026-11-13 |
| `run-43392807274-jobs.json` | `GET repos/nickderobertis/onetaskgraph/actions/runs/43392807274/jobs?per_page=100&page=1` (its skipped `lint` and `budgets` complete a second before they start) | 2026-11-13 |
| `rate-limit.json` | `GET rate_limit` | 2026-11-13 |
