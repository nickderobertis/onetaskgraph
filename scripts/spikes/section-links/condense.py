#!/usr/bin/env python3
"""Condense the section-links report's `method` answer so the report fits in a GitHub issue.

`report.py` writes one table row per demonstration link and every Linear item's whole stored
body, which composes to about 226,000 characters. GitHub refuses an issue body over 65,536, so
the plan's copy to the `plans` board refused the report. This keeps every finding and changes
what the method lists, by three rules:

- A form (the code's letter: `S`, `U`, `W`, …) that landed on any target of a destination, or
  that nobody has observed yet (Linear's rows, which await a click), keeps one row per heading.
- A form observed on a destination and landing on none of its targets is stated once per target
  and result: one representative clickable link and how many links were observed with it.
- A Linear item's stored body is shown as an excerpt (its anchor line and heading, or a source's
  first links) beside the link to the item, which holds it in full; its size and SHA-256 stay.

`report.py` applies `condense_method` to what it composes. Run alone, it condenses answers a
report already holds, so the stored report can be re-rendered without its run's output:

    python3 scripts/spikes/section-links/condense.py --answers IN.json --out OUT.json
"""

from __future__ import annotations

import argparse
import collections
import json
import pathlib
import re
import sys

ROW = re.compile(r"^\| (?P<link>.*?) \| (?P<heading>`[^`]*`) \| (?P<result>.*) \|$")
CODE = re.compile(r"(?P<code>[A-Z]{2}-[PIN]-(?P<form>[A-Z])\d)")
LINKED = re.compile(r"^\[(?P<code>[^\]]+)\]\((?P<href>[^)]+)\)")
ARROW = re.compile(r"→ `(?P<href>[^`]+)`")
STORED = re.compile(r"; stored as `(?P<href>[^`]+)` \(unchanged\)$")
REPEAT = "`## Budgets (the second heading with this text, at the end)`"
AWAITING = "**awaiting your click**"
COMPLETE_SECTION = "### Linear: complete stored bodies"
EXCERPT_SECTION = "### Linear: stored bodies (excerpts)"


def parse_row(line: str) -> dict | None:
    match = ROW.match(line)
    if not match or not CODE.search(match["link"]):
        return None
    code = CODE.search(match["link"])
    return {"line": line, "code": code["code"], "form": code["form"], **match.groupdict()}


def landed(row: dict) -> bool:
    return "✅" in row["result"]


def observed(row: dict) -> bool:
    return AWAITING not in row["result"]


def clickable(row: dict) -> str:
    """The row's link as a clickable Markdown link, whichever way the row spelled it."""
    linked = LINKED.match(row["link"])
    if linked:
        return f"[{linked['code']}]({linked['href']})"
    arrow = ARROW.search(row["link"])
    if arrow:
        return f"[{row['code']}]({arrow['href']})"
    sys.exit(f"condense: row {row['code']} holds no link to keep clickable: {row['line']}")


FORMS = {
    "S": "#<slug>",
    "U": "#user-content-<slug>",
    "A": "<absolute path>#<slug>",
    "B": "<absolute path>#user-content-<slug>",
}


def form_label(row: dict) -> str:
    """The form's own spelling: the code span after the link when it names a form, else the letter's."""
    linked = LINKED.match(row["link"])
    rest = row["link"][len(linked[0]):] if linked else row["link"]
    span = re.search(r"`([^`]*)`", rest)
    if span and span[1].startswith(("#", "<")) or span and ", " in span[1]:
        return span[1]
    return FORMS[row["form"]]


SHORT = (
    ("**lands** ✅", "lands ✅"),
    ("opens the record at its top ❌", "top ❌"),
    ("does not scroll: GitHub navigates in place and keeps the source issue's scroll offset ❌", "offset kept ❌"),
    ("GitHub answers *File not found* (404) ❌", "404 ❌"),
    ("no preview opens: the path resolves under the workspace folder to a file that does not exist ❌", "no preview ❌"),
)
LEGEND = (
    "**Reading the tables.** One row per heading, one column per form; each cell is the demonstration link (click "
    "it) and what Chromium did: `lands ✅` the named heading is at the top of the viewport; `top ❌` the record opens "
    "at its top; `offset kept ❌` GitHub navigated in place and kept the source issue's scroll offset; `y=<n>, no "
    "heading at top ❌` the page scrolled there with no heading at the top; `404 ❌` GitHub answers *File not found*; "
    "`no preview ❌` the path resolves under the workspace folder to a file that does not exist. A GitHub issue cell "
    "gives `opened` (a fresh tab loading the URL, as clicking it here does) and `clicked` (clicked inside the source "
    "issue). A result shared by a whole column is in its header. A form observed landing on none of a destination's "
    "targets is stated once per target, with one representative link and the number of links with that result."
)
SERVICE = re.compile(r" \(language service: (?P<answer>[^)]*)\)$")


def short(result: str) -> str:
    """A result in the legend's words."""
    result = SERVICE.sub("", result)
    for long, brief in SHORT:
        result = result.replace(long, brief)
    result = re.sub(r"scrolled to (y=\d+) with no heading at the top ❌", r"\1, no heading at top ❌", result)
    result = result.replace("opened directly: ", "opened ").replace("; clicked in the source: ", ", clicked ")
    for where in ("clicked in the file view: ", "clicked in the preview: "):
        result = result.replace(where, "")
    stored = STORED.search(result)
    if stored:
        result = result[: stored.start()]
    return result.replace(AWAITING, "awaiting your click")


def heading_label(heading: str) -> str:
    return "`## Budgets` (the repeat, at the end)" if heading == REPEAT else heading


def condense_table(rows: list[dict], keep_forms: set[str]) -> list[str]:
    for row in rows:
        stored = STORED.search(row["result"])
        if stored and (not LINKED.match(row["link"]) or LINKED.match(row["link"])["href"] != stored["href"]):
            sys.exit(f"condense: {row['code']} was stored unchanged but its link is not the stored URL")
    kept = [r for r in rows if r["form"] in keep_forms]
    lines: list[str] = []
    if kept:
        forms = list(dict.fromkeys(r["form"] for r in kept))
        label = {r["form"]: form_label(r) for r in kept}
        headings = list(dict.fromkeys(r["heading"] for r in kept))
        cells = {(r["heading"], r["form"]): r for r in kept}
        # A column whose every link had one result states it once, in its header.
        shared = {}
        for form in forms:
            results = {short(r["result"]) for r in kept if r["form"] == form}
            if len(results) == 1:
                shared[form] = results.pop()
        lines += ["| Must land on | " + " | ".join(
                      f"`{label[f]}`" + (f": every link {shared[f]}" if f in shared and observed(kept[0]) else "")
                      for f in forms) + " |",
                  "| --- |" + " --- |" * len(forms)]
        for heading in headings:
            row_cells = []
            for form in forms:
                r = cells.get((heading, form))
                text = "" if r is None else clickable(r) + ("" if form in shared else f" {short(r['result'])}")
                row_cells.append(text.replace("|", "\\|"))
            lines.append(f"| {heading_label(heading)} | " + " | ".join(row_cells) + " |")
        notes = []
        if not all(STORED.search(r["result"]) for r in kept) and not all(observed(r) for r in kept):
            sys.exit("condense: a Linear link here was not stored unchanged, which the section's intro would misstate")
        answers = collections.Counter(SERVICE.search(r["result"])["answer"] for r in kept if SERVICE.search(r["result"]))
        notes += [f"VS Code's language service answered *{a}* for {n} of them." for a, n in answers.items()]
        if notes:
            lines += ["", " ".join(notes)]
    collapsed: dict[tuple[str, str], list[dict]] = collections.OrderedDict()
    for row in rows:
        if row["form"] not in keep_forms:
            collapsed.setdefault((row["form"], row["result"]), []).append(row)
    if collapsed:
        lines += [""] if lines else []
        for (_, result), group in collapsed.items():
            first, count = group[0], len(group)
            service = SERVICE.search(result)
            lines.append(
                f"- `{form_label(first)}`, lands on no target here: {count} link{'s' if count != 1 else ''}, "
                f"{'each ' if count != 1 else ''}{short(result)}"
                + (f" (language service: *{service['answer']}*)" if service and service["answer"] not in short(result) else "")
                + f"; e.g. {clickable(first)}."
            )
    return lines


def destination_sections(lines: list[str]) -> list[tuple[int, int]]:
    """(start, end) line ranges of each `### ` section, passing over headings inside a fence."""
    starts, fence = [], None
    for i, line in enumerate(lines):
        opener = re.match(r"^(`{3,})", line)
        if fence is None and opener:
            fence = opener[1]
        elif fence is not None and line == fence:
            fence = None
        elif fence is None and line.startswith("### "):
            starts.append(i)
    return [(s, starts[k + 1] if k + 1 < len(starts) else len(lines)) for k, s in enumerate(starts)]


SELF_LINK = re.compile(r"\[(?P<label>https?://[^\]\s]+)\]\((?P=label)\)")


def self_label(url: str) -> str:
    """Short text for a link whose text was its own URL: its last segment, or `open it` when that is long."""
    last = url.rstrip("/").rsplit("/", 1)[-1]
    return last if len(last) <= 20 else "open it"


def short_heading(line: str) -> str:
    """A line whose links' text repeats their URL, each given a short text instead."""
    return SELF_LINK.sub(lambda m: f"[{self_label(m['label'])}]({m['label']})", line)


def condense_destination(lines: list[str]) -> list[str]:
    lines = [short_heading(line) for line in lines]
    rows = [r for r in (parse_row(line) for line in lines) if r]
    if not rows:
        return lines
    keep = {r["form"] for r in rows if landed(r) or not observed(r)}
    out: list[str] = []
    i = 0
    while i < len(lines):
        if lines[i].startswith("| ") and i + 1 < len(lines) and lines[i + 1].startswith("| ---"):
            j = i + 2
            table_rows = []
            while j < len(lines) and lines[j].startswith("| "):
                row = parse_row(lines[j])
                if row is None:
                    sys.exit(f"condense: a table row this script cannot read: {lines[j]}")
                table_rows.append(row)
                j += 1
            out += condense_table(table_rows, keep)
            i = j
            continue
        out.append(lines[i])
        i += 1
    return out


def excerpt(body: str) -> str:
    """The lines of a stored body that carry what the findings cite."""
    lines = body.splitlines()
    anchor = next((k for k, line in enumerate(lines) if line.startswith("<a ")), None)
    if anchor is not None:
        return "\n".join(lines[anchor : anchor + 3])
    links = [k for k, line in enumerate(lines) if line.startswith("* [")]
    if links:
        return "\n".join(lines[links[0] : links[0] + 2])
    heading = next(k for k, line in enumerate(lines) if line.startswith("## "))
    return "\n".join(lines[heading : heading + 3])


def condense_stored_bodies(lines: list[str]) -> list[str]:
    text = "\n".join(lines)
    blocks = re.split(r"(?m)^(?=#### Linear )", text)
    out = [
        EXCERPT_SECTION, "",
        "Each item below is shown by an excerpt of the `description` (issues) or `content` (documents) Linear's API "
        "returned when the harness read it back after creating it: the raw anchor line and the heading after it for "
        "a target (the first heading for a plain one), the first stored links for a source. The item at each link "
        "holds the whole stored body. Size and SHA-256 are of that whole returned text, UTF-8 encoded.", "",
    ]
    for block in blocks[1:]:
        title = short_heading(block.splitlines()[0])
        summary = re.search(r"(?m)^Stored `\w+`: .*$", block)
        body = re.search(r"(?s)\n(`{4,})markdown\n(?P<body>.*?)\n\1\n", block)
        if not summary or not body:
            sys.exit(f"condense: a stored-body block this script cannot read: {title}")
        fence = "`" * max(4, max((len(r) for r in re.findall(r"`+", body["body"])), default=0) + 1)
        out += [title, "", summary[0] + " Excerpt:", "", fence, excerpt(body["body"]), fence, ""]
    return out


def condense_method(method: str) -> str:
    lines = method.rstrip("\n").split("\n")
    sections = destination_sections(lines)
    out = (lines[: sections[0][0]] if sections else lines)
    if sections:
        out = [*out, LEGEND, ""]
    for start, end in sections:
        part = lines[start:end]
        if part[0] == COMPLETE_SECTION:
            out += condense_stored_bodies(part)
        else:
            out += condense_destination(part)
    text = "\n".join(out).rstrip() + "\n"
    # One pointer per Linear section, instead of one per item, to the excerpts.
    text = re.sub(
        r"\n\nThe complete `\w+` Linear stored for (this target|the source \w+)(, holding every link above as stored,)? "
        r"is under \*Linear: complete stored bodies\* below, as `[^`]+`\.",
        "",
        text,
    )
    text = text.replace(
        "Each row gives the link exactly as Linear stored it.",
        "Each cell's link is exactly what Linear stored: every link came back byte for byte. An excerpt of each item's stored body is under *Linear: "
        "stored bodies (excerpts)* below, and the item itself holds it in full.",
    )
    # The harness answer already says how the one command is run; the method need not say it twice.
    text = re.sub(r"\*\*The one command\.\*\*[^\n]*\n\n", "", text)
    text = text.replace(
        "Forms `A` and `B` resolve to a file that does not exist, so they are listed as text: click them inside ",
        "Forms `A` and `B` resolve to a file that does not exist: each one listed links the address GitHub resolved it "
        "to, which answers the 404, as clicking it inside ",
    ).replace(" to see the 404.", " does.")
    if "complete stored bodies" in text:
        sys.exit("condense: a reference to the complete stored bodies is left that this script did not rewrite")
    return text


def condense(answers: dict, provenance: str | None = None) -> dict:
    method = condense_method(answers["method"])
    if provenance:
        first, rest = method.split("\n\n", 1)
        method = f"{first} {provenance}\n\n{rest}"
    return {**answers, "method": method}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--answers", required=True, type=pathlib.Path)
    parser.add_argument("--out", required=True, type=pathlib.Path)
    parser.add_argument("--provenance", help="a sentence appended to the method's first paragraph naming where the "
                        "condensed answers came from")
    args = parser.parse_args()
    answers = json.loads(args.answers.read_text())
    args.out.write_text(json.dumps(condense(answers, args.provenance), indent=2, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
