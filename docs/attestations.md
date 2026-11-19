# Narrowing a project's attestation check, and `task list --not-status`

The filters a whole-project attestation check takes, and the `task list` flag that shares their
status vocabulary.

<!-- llmlint: ignore-block[contracts_have_one_source_or_a_drift_gate] Every claim below is held by the journeys in `crates/onetaskgraph-e2e/tests/e2e/attestation_filters.rs` — `each_filter_lists_exactly_the_tasks_it_keeps_in_listing_order`, `a_filter_without_tasks_is_refused_as_an_invocation` and `task_list_not_status_drops_the_categories_it_names` — which drive the binary and fail when the behaviour stated here moves. -->

## `project attestation check --tasks` and its three filters

```sh
onetaskgraph project attestation check <PROJECT-ID> --kind <KIND> --tasks \
  [--status CATEGORY]... [--not-status CATEGORY]... [--state STATE]... [--bar LABEL] [--json]
```

| Flag | Keeps a task when | Values |
| --- | --- | --- |
| `--status CATEGORY` | its status category is one of those named | `draft`, `backlog`, `todo`, `queued`, `in-progress`, `done`, `cancelled`, `unknown` |
| `--not-status CATEGORY` | its status category is none of those named | the same eight |
| `--state STATE` | its check answers one of the states named | `valid`, `void`, `absent` |

- **Each flag is repeatable, and repeats of one flag are alternatives**: `--status todo --status
  in-progress` keeps a task at either category.
- **Different flags intersect**: a task is listed only when it passes every flag given. So
  `--status done --state valid` lists the done tasks whose record holds, and `--status done
  --not-status done` lists nothing. Unlike `task list`, the check does not refuse that pair —
  it answers an empty list, which exits `0`.
- **A task its status leaves out is not read for its record.** The status flags are decided on
  the project's listing; `--state` is decided on each remaining task's check.
- **The filters need `--tasks`.** Given without it, each is refused as an invocation (exit `2`)
  naming `--tasks`.
- **No filter answers what the check always answered**: every task filed under the project.

### What it answers

The order is the order the source lists the project's tasks in — the unfiltered check's order,
with the tasks a filter leaves out removed and nothing reordered.

The JSON shape does not change with the filters. `--json` answers
`{"project", "kind", "tasks": [...]}`, each entry carrying `id`, `kind`, `state`, `reason` and
`record` exactly as the unfiltered check carries them; a filter only decides which entries are
in `tasks`.

The exit code is decided over the tasks listed, not the project's whole set:

| Exit | When |
| --- | --- |
| `0` | every task listed is `valid` — including when the filters list none |
| `3` | at least one task listed is `void` or `absent` |

So the work a review still owes is one call:

```sh
onetaskgraph project attestation check notes:plan-1 --kind plan-review --tasks \
  --not-status done --not-status cancelled --state void --state absent --json
```

It lists the open tasks holding no valid `plan-review` record, and exits `0` exactly when there
are none. Note that `--state valid` alone always exits `0`, because every task it lists is
valid; read `tasks` to see what it kept.

## `task list --not-status`

```sh
onetaskgraph task list [--status CATEGORY]... [--not-status CATEGORY]... ...
```

`--not-status` takes the same eight categories and drops the tasks at any it names, as
`--status` keeps the tasks at any it names. Alone, it keeps every task at a category it does
not name — `--not-status draft` over a project with no drafts lists every task. Beside
`--status`, a task is kept at a category `--status` names and `--not-status` does not.

A pair that leaves no category at all — `--status done --not-status done` — could match
nothing, so it is refused as `no-status-left`, exiting non-zero and naming `--not-status` on
standard error, rather than being read as no filter.

<!-- llmlint: ignore-end[contracts_have_one_source_or_a_drift_gate] -->
