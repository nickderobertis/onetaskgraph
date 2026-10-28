"""`task_locate` and `sources_route`'s `reachable`, against the real binary over folders.

`plan` is where the tasks are written; `site` is where they are located and copied to, and it
routes `github.com/example-org/*` work to `team`. Every answer is what the binary printed,
validated against the models generated from its own schema.
"""

from __future__ import annotations

import asyncio
import json
import os
import sys
from pathlib import Path

import pytest

from onetaskgraph_sdk import Client, OnetaskgraphError
from onetaskgraph_sdk._generated.copy_report import CopyOutcomeCreatedOrWouldCreate
from onetaskgraph_sdk._generated.locate_report import CopyVia, LocatedCounterpart


def record(store: Path, folder: str, name: str, front: str = "") -> None:
    """Write the task `name` into `folder`, with `front` beside its title and status."""
    tasks = store / folder / "tasks"
    tasks.mkdir(parents=True, exist_ok=True)
    (tasks / f"{name}.md").write_text(
        f"---\ntitle: {name}\nstatus: todo\n{front}\n---\nBody of {name}.\n",
        encoding="utf-8",
        newline="\n",
    )


@pytest.fixture
def client(binary: Path, tmp_path: Path) -> Client:
    """A client over `plan`, `site` and `team`, reading no configuration but this store's."""
    record(tmp_path, "plan", "fresh")
    # A link naming an item `site` no longer holds, and the item there that names it back.
    record(tmp_path, "plan", "pinned", 'metadata: {onetaskgraph.copies: {site: "site:gone"}}')
    record(tmp_path, "site", "held", 'metadata: {onetaskgraph.origin: "plan:pinned"}')
    (tmp_path / "team").mkdir()
    sources: dict[str, dict[str, object]] = {
        folder: {"plugin": "local-md", "config": {"root": str(tmp_path / folder)}}
        for folder in ("plan", "site", "team")
    }
    sources["site"]["routes"] = [{"repositories": ["github.com/example-org/*"], "to": "team"}]
    (tmp_path / "onetaskgraph.yaml").write_text(json.dumps({"sources": sources}), encoding="utf-8")
    environment = {**os.environ, "HOME": str(tmp_path), "XDG_CONFIG_HOME": str(tmp_path)}
    return Client(binary, cwd=tmp_path, environment=environment)


def test_a_plain_locate_answers_the_placement_and_the_counterpart_the_next_copy_uses(
    client: Client,
) -> None:
    """Never copied, a task has no counterpart; once copied, its link finds the item."""
    [fresh] = asyncio.run(client.task_locate(["plan:fresh"], to="site")).items
    assert fresh.source.root == "plan:fresh"
    assert fresh.classification == "public"
    assert (fresh.destination.root, fresh.route) == ("site", None)
    assert fresh.counterpart is None
    assert fresh.carriers == []
    assert fresh.creates_in is None

    copied = asyncio.run(client.task_copy(["plan:fresh"], to="site"))
    landed = copied.items[0].root
    assert isinstance(landed, CopyOutcomeCreatedOrWouldCreate)
    assert landed.destination is not None
    [linked] = asyncio.run(client.task_locate(["plan:fresh"], to="site")).items
    assert linked.counterpart is not None
    assert linked.counterpart.model_dump(mode="json") == {
        "id": landed.destination.root,
        "via": "link",
    }

    # `--no-counterpart` looks nothing up, so the answer names neither.
    [placed] = asyncio.run(client.task_locate(["plan:fresh"], to="site", no_counterpart=True)).items
    assert placed.carriers is None
    assert "counterpart" not in placed.model_dump(exclude_unset=True)


def test_target_native_answers_the_named_item_while_the_task_s_link_is_stale(
    client: Client,
) -> None:
    """A plain locate refuses the stale link; the named counterpart does not follow it."""
    with pytest.raises(OnetaskgraphError) as stale:
        asyncio.run(client.task_locate(["plan:pinned"], to="site"))
    assert stale.value.failure is not None
    assert stale.value.failure.kind == "stale-link"

    [pinned] = asyncio.run(
        client.task_locate(["plan:pinned"], to="site", target_native="held")
    ).items
    assert pinned.counterpart == LocatedCounterpart(id="site:held", via=CopyVia.CopyViaTargetTask)
    assert pinned.carriers == []


@pytest.mark.parametrize(
    ("ids", "no_counterpart", "kind"),
    [
        (("plan:pinned",), True, "target-native-with"),
        (("plan:pinned", "plan:fresh"), None, "target-native-needs-one"),
    ],
)
def test_target_native_is_refused_beside_no_counterpart_and_beside_two_ids(
    client: Client, ids: tuple[str, ...], no_counterpart: bool | None, kind: str
) -> None:
    """Each refusal is the SDK's error carrying the binary's own failure kind."""
    with pytest.raises(OnetaskgraphError) as refused:
        asyncio.run(
            client.task_locate(ids, to="site", target_native="held", no_counterpart=no_counterpart)
        )
    assert refused.value.exit_code == 1
    assert refused.value.failure is not None
    assert refused.value.failure.kind == kind


def test_sources_route_names_every_source_a_source_reaches(client: Client) -> None:
    """The source itself first, then each route target; the same whatever the item names."""
    for repositories in (None, ["github.com/example-org/app"]):
        route = asyncio.run(client.sources_route("site", repository=repositories))
        assert [name.root for name in route.reachable] == ["site", "team"]
    assert [name.root for name in asyncio.run(client.sources_route("team")).reachable] == ["team"]


@pytest.mark.parametrize("written", ["", "not a document", '{"failure": {"kind": 7}}'])
def test_a_refusal_whose_output_is_no_failure_document_carries_no_failure(
    tmp_path: Path, written: str
) -> None:
    """Only a document the binary's schema describes becomes the error's `failure`."""
    # The interpreter as the binary: it runs the file named by the client's first word, `task`,
    # from the working directory, which answers as a refusing binary would on every platform.
    (tmp_path / "task").write_text(
        f"import sys\nsys.stdout.write({written!r})\nsys.stderr.write('refused')\nsys.exit(1)\n",
        encoding="utf-8",
    )
    with pytest.raises(OnetaskgraphError) as refused:
        asyncio.run(Client(sys.executable, cwd=tmp_path).task_locate(["plan:fresh"], to="site"))
    assert (refused.value.exit_code, str(refused.value)) == (1, "refused")
    assert refused.value.failure is None
