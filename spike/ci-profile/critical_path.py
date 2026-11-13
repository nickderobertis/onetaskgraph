"""How much sooner a job ends when some of its tasks finish sooner: the one rule both replays share.

A job's timeline is its executed Nx tasks' intervals. Nx's report carries no dependency edges, so
the critical path is jobprofile's latest-finishing chain: walking back from the last end, the task
finishing latest before each pointer. A changed task on that chain moves the chain's end, every
later link moving with it; a changed task off the chain moves only its own end; the job ends at the
latest of the two. Off-chain followers are not shifted and contention is not modelled.
"""

from __future__ import annotations

SHORTEST_LINK = 0.5  # seconds: as jobprofile's chain, a replay holds no slot worth chaining

Interval = tuple[float, float]


def latest_chain(intervals: dict[str, Interval]) -> list[str]:
    """Walking back from the last end, the task finishing latest before each pointer (jobprofile's rule)."""
    pool = {t: iv for t, iv in intervals.items() if iv[1] - iv[0] > SHORTEST_LINK}
    if not pool:
        return []
    pointer, chain = max(e for _, e in pool.values()), []
    while before := [t for t, (s, e) in pool.items() if e <= pointer + 1 and s < pointer - 0.5]:
        link = max(before, key=lambda t: (pool[t][1], -pool[t][0]))
        chain.append(link)
        pointer = pool.pop(link)[0]
    return list(reversed(chain))


def span(intervals: dict[str, Interval], chain: list[str] | None = None) -> float:
    picked = [intervals[t] for t in chain] if chain is not None else list(intervals.values())
    return max(e for _, e in picked) - min(s for s, _ in picked) if picked else 0.0


def predicted_shortening(intervals: dict[str, Interval], changed: dict[str, float]) -> float:
    """How much sooner the job's last task ends when each changed task finishes `changed[t]` sooner.

    A changed task the timeline does not hold (a new crate's task) runs beside the gate from its start."""
    if not intervals:
        return 0.0
    end = max(e for _, e in intervals.values())
    first = min(s for s, _ in intervals.values())
    chain = set(latest_chain(intervals))
    chain_end = end - sum(v for t, v in changed.items() if t in chain)
    others = [e - changed.get(t, 0.0) for t, (_, e) in intervals.items() if t not in chain]
    added = [first - v for t, v in changed.items() if t not in intervals]
    return end - max([chain_end, *others, *added])
