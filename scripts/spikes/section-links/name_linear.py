#!/usr/bin/env python3
"""Name every Linear item in the section-links report instead of linking it.

A Linear URL carries the workspace's name (`linear.app/<workspace>/issue/...`), and the
workspace is private, so the public `plans` board refuses a report holding one. This replaces
each Linear URL with a plain reference to what it names, keeping the fragment it carried:

- an issue by its team key and number: `TES2-773`, or `` `TES2-773#what` `` with a fragment;
- a document by its title: *Section-links spike 20261102-173739: target document (plain)*,
  followed by `` `#what` `` with a fragment;
- Linear's public documentation by the page's name.

A demonstration link keeps its code (`LI-P-S1 `TES2-773#what``); a link whose text was its item
or `open it` becomes the reference alone; any other text keeps its words beside the reference.
Inside a fence, where the report quotes what Linear stored, `<URL>` becomes `<reference>`. The
method says once that the clickable Linear links reach the user from the manager, outside the
public board. A Linear URL this script cannot name stops it, naming the URL.

`report.py` applies `name_linear` after `condense`. Run alone, it names the Linear items in
answers a report already holds:

    python3 scripts/spikes/section-links/name_linear.py --answers IN.json --out OUT.json
"""

from __future__ import annotations

import argparse
import json
import pathlib
import re
import sys

LINEAR = r"https://linear\.app/[^\s)>\]`|]+"
ISSUE = re.compile(r"^https://linear\.app/[^/]+/issue/(?P<key>[A-Z][A-Z0-9]*-\d+)(?:/[^#]*)?(?P<frag>#[^\s]*)?$")
DOCUMENT = re.compile(
    r"^https://linear\.app/[^/]+/document/section-links-spike-(?P<stamp>\d{8}-\d{6})-"
    r"(?P<role>target|source)-document(?:-(?P<variant>plain|anchor-id|anchor-name))?-[0-9a-f]{12}(?P<frag>#[^\s]*)?$"
)
DOCS = {"https://linear.app/docs/editor": "the *Editor* page of Linear's public documentation"}
MD_LINK = re.compile(r"\[(?P<text>[^\]]*)\]\(<?(?P<url>" + LINEAR + r")>?\)")
BARE = re.compile(LINEAR)
CODE = re.compile(r"^[A-Z]{2}-[PIN]-[A-Z]\d$")
FENCE = re.compile(r"^(`{3,})")

CLICKABLE = (
    "A Linear cell or heading names its item rather than linking it — an issue by its TES2 key, a document by its "
    "title — with the fragment its link carried: a Linear URL names the private workspace, so the clickable Linear "
    "links are given to you by the manager, outside the public board."
)
REWORDED = (
    ("every link below is a real demonstration you can click,",
     "every link below is a real demonstration you can click (a Linear one is named, not linked; see *Reading the "
     "tables*),"),
    ("Each cell's link is exactly what Linear stored: every link came back byte for byte.",
     "Each cell names the link exactly as Linear stored it: every link came back byte for byte."),
    ("The item at each link holds the whole stored body.",
     "The item each heading names holds the whole stored body. In an excerpt each stored Linear URL is shown as "
     "`<item#fragment>`, the item and fragment it names."),
    ("Click the TES2 links in the Linear issue and Linear document tables above.",
     "Click the Linear links to the items and fragments the Linear issue and Linear document tables above name."),
)


def item(url: str) -> tuple[str, str, str]:
    """(kind, name, fragment) of a Linear item URL; stops on one this script cannot name."""
    issue = ISSUE.match(url)
    if issue:
        return "issue", issue["key"], issue["frag"] or ""
    doc = DOCUMENT.match(url)
    if doc:
        what = f"target document ({doc['variant']})" if doc["role"] == "target" else "source document"
        if (doc["role"] == "target") != bool(doc["variant"]):
            sys.exit(f"name_linear: a Linear document URL this script cannot name: {url}")
        return "document", f"Section-links spike {doc['stamp']}: {what}", doc["frag"] or ""
    sys.exit(f"name_linear: a Linear URL this script cannot name: {url}")


def reference(url: str) -> str:
    """The Markdown reference for a Linear URL outside a fence."""
    if url in DOCS:
        return DOCS[url]
    kind, name, frag = item(url)
    if kind == "issue":
        return f"`{name}{frag}`" if frag else name
    return f"*{name}*" + (f" `{frag}`" if frag else "")


def plain(url: str) -> str:
    """The reference for a Linear URL inside a fence, where nothing is rendered."""
    if url in DOCS:
        return DOCS[url]
    _, name, frag = item(url)
    return name + frag


def named_link(match: re.Match) -> str:
    text, url = match["text"], match["url"]
    ref = reference(url)
    if url in DOCS or text == "open it" or text == url or text == item(url)[1]:
        return ref
    return f"{text} {ref}"


def outside_fence(line: str) -> str:
    line = MD_LINK.sub(named_link, line)
    # A bare URL in prose, as `(https://linear.app/docs/editor)` is written.
    return BARE.sub(lambda m: reference(m[0]), line)


def name_text(text: str) -> str:
    out, fence = [], None
    for line in text.split("\n"):
        opener = FENCE.match(line)
        if fence is None and opener:
            fence = opener[1]
            out.append(line)
        elif fence is not None:
            if line == fence:
                fence = None
                out.append(line)
            else:
                out.append(re.sub(r"<(" + LINEAR + r")>", lambda m: f"<{plain(m[1])}>", line))
        else:
            out.append(outside_fence(line))
    return "\n".join(out)


def name_linear(answers: dict, provenance: str | None = None) -> dict:
    method = name_text(answers["method"])
    findings = [name_text(f) for f in answers["findings"]]
    if "linear.app" in method or any("linear.app" in f for f in findings):
        sys.exit("name_linear: a Linear URL is left that this script did not name")
    for old, new in REWORDED:
        method = method.replace(old, new)
        findings = [f.replace(old, new) for f in findings]
    if "### Linear" in method:
        legend = re.search(r"(?m)^\*\*Reading the tables\.\*\*.*$", method)
        if not legend:
            sys.exit("name_linear: the method has no *Reading the tables* paragraph to say where the Linear links are")
        method = method[: legend.end()] + " " + CLICKABLE + method[legend.end():]
    if provenance:
        first, rest = method.split("\n\n", 1)
        method = f"{first} {provenance}\n\n{rest}"
    return {**answers, "method": method, "findings": findings}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--answers", required=True, type=pathlib.Path)
    parser.add_argument("--out", required=True, type=pathlib.Path)
    parser.add_argument("--provenance", help="a sentence appended to the method's first paragraph naming where the "
                        "answers came from")
    args = parser.parse_args()
    answers = json.loads(args.answers.read_text())
    args.out.write_text(json.dumps(name_linear(answers, args.provenance), indent=2, ensure_ascii=False) + "\n")


if __name__ == "__main__":
    main()
