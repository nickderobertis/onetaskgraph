"""The board-copy-gaps feasibility spike.

Live probes of the test board, synthetic reachability and screening cases against the real CLI
and the real onevcs, and the analysis that turns their sanitized logs into the spike report.
It extends the timing spike's harness under `spike/harness/` (its 40% quota rule, its workload
generator and its leak scan) without changing it; live requests leave through `gaps.guard`.
"""
