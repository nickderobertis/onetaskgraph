#!/usr/bin/env python3
"""Tests of `name_linear.py`, run as a subprocess over answers shaped as `condense.py` writes them.

    python3 scripts/spikes/section-links/test_name_linear.py
"""

from __future__ import annotations

import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

NAME_LINEAR = pathlib.Path(__file__).resolve().parent / "name_linear.py"
WS = "https://linear.app/someworkspace"
ISSUE = f"{WS}/issue/TES2-773"
DOC = f"{WS}/document/section-links-spike-20261102-173739-target-document-anchor-id-c66bc769db58"
SOURCE_DOC = f"{WS}/document/section-links-spike-20261102-173739-source-document-7e76b7d9590e"
GITHUB = "[GI-P-S1](https://github.com/o/r/issues/1#what)"
TITLE = "Section-links spike 20261102-173739: target document (anchor-id)"


def answers(extra_line: str = "") -> dict:
    method = "\n".join([
        "This is the acceptance document: every link below is a real demonstration you can click, beside the heading.",
        "",
        "**Reading the tables.** One row per heading.",
        "",
        f"- `#<slug>`, lands on no target here: 5 links; e.g. {GITHUB}.",
        "",
        "### Linear issue",
        "",
        f"Source issue: [TES2-776]({WS}/issue/TES2-776). Each cell's link is exactly what Linear stored: every link "
        "came back byte for byte.",
        "",
        f"#### Links to the plain target: [TES2-773]({ISSUE})",
        "",
        "| Must land on | `#<slug>` |",
        "| --- | --- |",
        f"| `## What` | [LI-P-S1]({ISSUE}#what) |",
        f"| `## Budgets` | [LD-I-S2]({DOC}#budgets) |",
        "",
        f"#### Linear document source: [open it]({SOURCE_DOC})",
        "",
        "The item at each link holds the whole stored body.",
        "",
        "````",
        f"* [LI-P-S1 What](<{ISSUE}#what>) — must land on `## What`",
        "````",
        extra_line,
    ])
    findings = [
        f"**Linear.** Stored as sent ([source issue]({WS}/issue/TES2-776), [`<a id>` target]({DOC})).",
        "Linear's own editor documentation (https://linear.app/docs/editor) says nothing. Click the TES2 links in the "
        "Linear issue and Linear document tables above.",
        "**A GitHub finding.** Unchanged, " + GITHUB + ".",
    ]
    return {"spike": "s", "method": method, "findings": findings, "candidates": []}


def run(given: dict) -> tuple[subprocess.CompletedProcess, dict | None]:
    folder = pathlib.Path(tempfile.mkdtemp())
    (folder / "in.json").write_text(json.dumps(given))
    ran = subprocess.run([sys.executable, str(NAME_LINEAR), "--answers", str(folder / "in.json"), "--out",
                          str(folder / "out.json"), "--provenance", "Named from stored answers."],
                         capture_output=True, text=True, timeout=60)
    out = folder / "out.json"
    return ran, json.loads(out.read_text()) if out.exists() else None


class NameLinearTest(unittest.TestCase):
    def test_every_linear_url_becomes_a_reference_to_its_item_and_fragment(self):
        ran, out = run(answers())
        self.assertEqual(ran.returncode, 0, ran.stderr)
        text = json.dumps(out, ensure_ascii=False)
        self.assertNotIn("linear.app", text)
        self.assertNotIn("someworkspace", text)
        method = out["method"]
        self.assertIn("Source issue: TES2-776.", method)
        self.assertIn("#### Links to the plain target: TES2-773\n", method)
        self.assertIn("| `## What` | LI-P-S1 `TES2-773#what` |", method)
        self.assertIn(f"| `## Budgets` | LD-I-S2 *{TITLE}* `#budgets` |", method)
        self.assertIn("#### Linear document source: *Section-links spike 20261102-173739: source document*\n", method)
        self.assertIn("* [LI-P-S1 What](<TES2-773#what>) — must land on `## What`", method)
        self.assertNotIn("](TES2", method)
        # GitHub links are untouched; the method says once where the clickable Linear links are.
        self.assertIn(GITHUB, method)
        self.assertEqual(out["findings"][2], answers()["findings"][2])
        self.assertEqual(method.count("given to you by the manager, outside the public board"), 1)
        self.assertIn("**Reading the tables.** One row per heading. A Linear cell", method)
        self.assertTrue(method.split("\n\n")[0].endswith("Named from stored answers."))
        self.assertIn("Each cell names the link exactly as Linear stored it", method)
        self.assertIn(f"(source issue TES2-776, `<a id>` target *{TITLE}*)", out["findings"][0])
        self.assertIn("(the *Editor* page of Linear's public documentation)", out["findings"][1])
        self.assertIn("Click the Linear links to the items and fragments", out["findings"][1])

    def test_a_linear_url_it_cannot_name_stops_it_naming_the_url(self):
        ran, out = run(answers(f"See [this]({WS}/project/some-project-abc)."))
        self.assertNotEqual(ran.returncode, 0)
        self.assertIn("cannot name", ran.stderr)
        self.assertIn("some-project-abc", ran.stderr)
        self.assertIsNone(out)


if __name__ == "__main__":
    unittest.main()
