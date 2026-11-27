"""How much sooner a job ends when some of its tasks finish sooner, over Nx's real task dependencies.

A job's timeline is its executed Nx tasks' observed intervals, and its dependencies are Nx's own
task graph for those tasks (`taskgraphs.py`, read at the run's commit), followed through any task
the run did not execute. Nothing is inferred from timestamps alone: two tasks that ran one after
the other without an edge between them are independent.

The replay re-times the observed run. A task whose observed start came within `WAIT_SLACK` of the
end of one of its dependencies was waiting on its dependencies: it starts again when its
dependencies end. Any other task was waiting on something the graph does not hold — a free slot
in its Nx invocation, a phase barrier in `scripts/ci-job.sh`, the runner's cores — and keeps its
observed start, unless a dependency now ends later. So a change moves only along edges the run
really waited on; a slot a shortened task frees is never handed to anyone (which can only
under-state a saving), and contention between concurrent tasks is not modelled. Replayed with no
change it reproduces the observed job to within `WAIT_SLACK` per waited edge, and a prediction is
the difference between that replay and the replay with the change, so the slack cancels. It
remains a model: its figures are estimates, beside the measured job times of matched hosted runs.
"""

from __future__ import annotations

WAIT_SLACK = 2.0  # seconds: Nx starts a dependent within moments of its last dependency ending

Interval = tuple[float, float]


def executed_deps(graph: dict[str, list[str]], executed: set[str]) -> dict[str, set[str]]:
    """Each executed task's dependencies among the executed tasks, followed through any task not executed."""
    out: dict[str, set[str]] = {}
    for task in executed:
        found: set[str] = set()
        seen: set[str] = set()
        stack = list(graph.get(task, []))
        while stack:
            dep = stack.pop()
            if dep in seen or dep == task:
                continue
            seen.add(dep)
            if dep in executed:
                found.add(dep)
            else:
                stack.extend(graph.get(dep, []))
        out[task] = found
    return out


def replay_end(intervals: dict[str, Interval], deps: dict[str, set[str]], changed: dict[str, float]) -> float:
    """The job's last end when each changed task takes `changed[t]` seconds less (more, if negative).

    A changed task the timeline does not hold (a new crate's task) runs from the gate's start, with no
    dependency."""
    if not intervals and not changed:
        return 0.0
    first = min((s for s, _ in intervals.values()), default=0.0)
    end: dict[str, float] = {}
    for task in sorted(intervals, key=lambda t: (intervals[t][0], intervals[t][1])):
        start, finish = intervals[task]
        before = [d for d in deps.get(task, set()) if d in intervals]
        waited = any(abs(start - intervals[d][1]) <= WAIT_SLACK for d in before)
        ready = max((end[d] if d in end else intervals[d][1] for d in before), default=first)
        new_start = ready if waited else max(start, ready)
        end[task] = new_start + (finish - start) - changed.get(task, 0.0)
    added = [first - v for t, v in changed.items() if t not in intervals]
    return max([*end.values(), *added])


def predicted_shortening(intervals: dict[str, Interval], deps: dict[str, set[str]], changed: dict[str, float]) -> float:
    """How much sooner the job ends with the change than the same model without it (seconds)."""
    if not changed:
        return 0.0
    return replay_end(intervals, deps, {}) - replay_end(intervals, deps, changed)


def gate_span(intervals: dict[str, Interval]) -> float:
    """First task start to last task end: the measured span of the Nx gate."""
    if not intervals:
        return 0.0
    return max(e for _, e in intervals.values()) - min(s for s, _ in intervals.values())
