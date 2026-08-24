# onetaskgraph-sdk

The typed Python client for `onetaskgraph`. Its request vocabulary, response models,
and method surface are generated from the JSON Schema and command help emitted by the
real binary; the workspace's `just lint` rejects committed output that has drifted.

```python
import asyncio
from onetaskgraph_sdk import Client

tasks = asyncio.run(Client().task_list(status=["todo"]))
```

Each async call runs one binary subprocess and validates its JSON response. The executable is
resolved in this order: the `binary=` constructor argument, the
`ONETASKGRAPH_SDK_BINARY` environment variable, then the `onetaskgraph` executable supplied
on `PATH` by the packaged binary distribution. Pass `cwd=` to select the directory from
which configuration is discovered. Partial query exit status 4 is parsed and returned,
so callers can inspect each typed `SourceFailure`; other non-zero statuses raise
`OnetaskgraphError` with the exit code.
