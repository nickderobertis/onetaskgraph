#!/usr/bin/env python3
"""Tests of `condense.py`, run as a subprocess over answers shaped as `report.py` writes them.

    python3 scripts/spikes/section-links/test_condense.py
"""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

CONDENSE = pathlib.Path(__file__).resolve().parent / "condense.py"
HEADINGS = ["`## What`", "`## Budgets`", "`### Budget demo-quota-seconds`", "`### Step 2.b: retry (once)`",
            "`## Budgets (the second heading with this text, at the end)`"]
SLUGS = ["what", "budgets", "budget-demo-quota-seconds", "step-2b-retry-once", "budgets-1"]
TOP = "opened directly: opens the record at its top ❌; clicked in the source: opens the record at its top ❌"
LANDS = "opened directly: **lands** ✅; clicked in the source: opens the record at its top ❌"
BODY = "**Target.**\n\nFiller before.\n\n<a id=\"what\"></a>\n\n## What\n\n" + "Filler that only the whole body holds.\n" * 50


def github_target(issue: int, variant: str, u_result: str) -> list[str]:
    url = f"https://github.com/o/r/issues/{issue}"
    rows = [f"| [GI-{variant}-S{k + 1}]({url}#{s}) `#<slug>` | {h} | {TOP} |" for k, (s, h) in enumerate(zip(SLUGS, HEADINGS))]
    rows += [f"| [GI-{variant}-U{k + 1}]({url}#user-content-{s}) `#user-content-<slug>` | {h} | {u_result} |"
             for k, (s, h) in enumerate(zip(SLUGS, HEADINGS))]
    return [f"#### Links to the target {variant}: [{url}]({url})", "", "| Link (click it) | Must land on | Observed in Chromium |",
            "| --- | --- | --- |", *rows, ""]


def linear_target(stored_suffix: str = " (unchanged)") -> list[str]:
    url = "https://linear.example/issue/T-1"
    rows = [f"| [LI-P-S{k + 1}]({url}#{s}) `#<slug>` | {h} | **awaiting your click**; stored as `{url}#{s}`{stored_suffix} |"
            for k, (s, h) in enumerate(zip(SLUGS, HEADINGS))]
    return [
        "### Linear issue", "", "Linear intro. Each row gives the link exactly as Linear stored it.", "",
        f"#### Links to the plain target: [{url}]({url})", "", "| Link (click it) | Must land on | Result |", "| --- | --- | --- |",
        *rows, "",
        "The complete `description` Linear stored for this target is under *Linear: complete stored bodies* below, as "
        "`issue target (plain)`.", "",
        "### Linear: complete stored bodies", "", "Intro.", "",
        f"#### Linear issue target (anchor-id): [{url}]({url})", "",
        "Stored `description`: 1 bytes, SHA-256 `ab`, different from the Markdown the harness sent.", "",
        "<details>", "<summary>Complete stored description</summary>", "", "````markdown", BODY.rstrip("\n"), "````", "",
        "</details>", "",
    ]


def answers(linear: list[str]) -> dict:
    method = "\n".join(["Preamble.", "", "### GitHub issue", "", "Intro.", "",
                        *github_target(1, "P", TOP), *github_target(2, "I", LANDS), *linear])
    return {"spike": "s", "method": method, "findings": ["**A finding.** Unchanged."], "candidates": []}


def run(given: dict) -> tuple[subprocess.CompletedProcess, dict | None]:
    folder = pathlib.Path(tempfile.mkdtemp())
    (folder / "in.json").write_text(json.dumps(given))
    ran = subprocess.run([sys.executable, str(CONDENSE), "--answers", str(folder / "in.json"), "--out", str(folder / "out.json"),
                          "--provenance", "Condensed from stored answers."], capture_output=True, text=True, timeout=60)
    out = folder / "out.json"
    return ran, json.loads(out.read_text()) if out.exists() else None


class CondenseTest(unittest.TestCase):
    def test_keeps_a_form_that_landed_somewhere_and_collapses_one_that_landed_nowhere(self):
        ran, out = run(answers(linear_target()))
        self.assertEqual(ran.returncode, 0, ran.stderr)
        method = out["method"]
        self.assertEqual(out["findings"], ["**A finding.** Unchanged."])
        self.assertTrue(method.startswith("Preamble. Condensed from stored answers."))
        # `U` landed on target I, so it keeps a clickable cell per heading on both targets.
        for variant, issue in (("P", 1), ("I", 2)):
            for k, slug in enumerate(SLUGS):
                self.assertIn(f"[GI-{variant}-U{k + 1}](https://github.com/o/r/issues/{issue}#user-content-{slug})", method)
        self.assertIn("`#user-content-<slug>`: every link opened top ❌, clicked top ❌", method)
        # `S` landed nowhere: one line per target, with its count and one representative link.
        self.assertIn("- `#<slug>`, lands on no target here: 5 links, each opened top ❌, clicked top ❌; "
                      "e.g. [GI-P-S1](https://github.com/o/r/issues/1#what).", method)
        self.assertNotIn("GI-P-S2", method)
        self.assertEqual(method.count("lands on no target here"), 2)
        # Linear's rows await a click, so every one is kept; a stored body is an excerpt only.
        for k in range(5):
            self.assertIn(f"[LI-P-S{k + 1}](", method)
        self.assertIn('<a id="what"></a>\n\n## What', method)
        self.assertNotIn("Filler that only the whole body holds.", method)
        self.assertNotIn("complete stored bodies", method)

    def test_a_linear_link_stored_changed_stops_rather_than_claiming_it_came_back_unchanged(self):
        ran, out = run(answers(linear_target(" (changed)")))
        self.assertNotEqual(ran.returncode, 0)
        self.assertIn("not stored unchanged", ran.stderr)
        self.assertIsNone(out)


if __name__ == "__main__":
    unittest.main()
