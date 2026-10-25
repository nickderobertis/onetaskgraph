# What Linear does to Markdown on write, one sample per measured rewrite

Each `.md` file here is one probe of spike `spike-linear-stored-content`, exactly as that spike's
harness wrote it to real Linear (`probes` in
`crates/onetaskgraph-linear/tests/spike_stored_content.rs` on the spike's branch,
`nick/linear-stored-content-verification/spike-linear-stored-content`), and the two images are
that harness's 73-byte PNGs. The measurements are its report,
`linear-stored-content-verification-spike-linear-stored-content-report` — the table *What Linear
stored, against what was sent* — taken on 2026-10-24 against the scratch team `TES`. Every
sample is one Linear rewrote:

| Sample | Where Linear's stored form first parts from what was sent (report, line: sent → stored) |
| --- | --- |
| `pipe-table.md` | line 2: `\| --- \| --- \|` → `\| -- \| -- \|`; and the table's last row keeps its line break, and the metadata slot after it is set off by one line break rather than two |
| `bullet-list.md` | line 1: `- first item` → `* first item` |
| `indented-code.md` | line 3: `    indented code line one` → a fence, ```` ``` ```` |
| `code-span-wrap.md` | line 1: the code span wrapped across two lines joined onto one |
| `bold-wrap.md` | line 1: `**bold text that spans` → `**bold text that spans**`, one run per line |
| `lazy-ordered-item.md` | line 1: `- a list item…` → `* a list item…`, and the `3. ` continuation escaped |
| `list-after-heading.md` | line 2: `- first item` → a blank line set before the list |
| `list-after-paragraph.md` | line 2: `- first item` → a blank line set before the list |
| `adjacent-images.md` | line 2: the second image → a blank line set between the two |

The loopback Linear of the shared e2e harness stores what it is written as
`onetaskgraph_e2e_support::linear_markdown::stored` gives it, and
`linear_rewrites.rs` in this suite holds that stand-in to every row above. The journeys compose
a realistic plan task and design document from these samples at test time, so no large body is
committed: the 43 KB rendered plan task the spike also probed is exercised against real Linear by
the spike's own harness and by the host's routed-plan journey, which renders the host's
templates when it runs.
