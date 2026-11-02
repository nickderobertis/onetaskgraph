#!/usr/bin/env python3
"""Write the section-links spike report's template answers from one harness run.

Reads `manifest.json` and `observations.json` from the harness's output directory and writes
`answers.json` beside them, the answers of the `spike-report` template: `method` is the
acceptance document (every demonstration link, the heading it must land on, and what was
observed), and `findings` are the conclusions. Every finding is checked against the
observations before it is written: when the data says otherwise this script stops naming the
finding, rather than writing a claim the run does not back.

    python3 scripts/spikes/section-links/report.py --out <harness output directory>
"""

from __future__ import annotations

import argparse
import collections
import json
import pathlib
import subprocess
import sys

HERE = pathlib.Path(__file__).resolve().parent
BRANCH = "nick/section-links/spike-section-links"
VARIANT_TITLE = {
    "plain": "plain target (no anchor markup)",
    "anchor-id": 'target with `<a id="<slug>"></a>` before each heading',
    "anchor-name": 'target with `<a name="<slug>"></a>` before each heading',
}
LANDS = "LANDS on the named heading"


def short(verdict: str) -> str:
    if verdict == LANDS:
        return "**lands** ✅"
    if verdict.startswith("stays at the source page's scroll offset"):
        return "does not scroll: GitHub navigates in place and keeps the source issue's scroll offset ❌"
    if verdict == "stays at the top of the page":
        return "opens the record at its top ❌"
    if verdict.startswith("opens nothing"):
        return "no preview opens: the path resolves under the workspace folder to a file that does not exist ❌"
    if "File not found" in verdict:
        return "GitHub answers *File not found* (404) ❌"
    return verdict + " ❌"


def cell(text: str) -> str:
    return text.replace("|", "\\|").replace("\n", " ")


def group(results: list[dict], mode: str) -> dict[tuple[str, str], collections.Counter]:
    """(variant, form) -> Counter of verdicts for one observation mode."""
    out: dict[tuple[str, str], collections.Counter] = collections.defaultdict(collections.Counter)
    for entry in results:
        if mode in entry:
            out[(entry["variant"], entry["form"])][entry[mode]["verdict"]] += 1
    return out


def only(groups, variant: str, form: str, predicate, what: str) -> None:
    """Stop unless every verdict of (variant, form) satisfies `predicate`."""
    verdicts = groups.get((variant, form))
    if not verdicts or not all(predicate(v) for v in verdicts):
        sys.exit(f"the observations do not back the finding '{what}': {variant} {form} -> {dict(verdicts or {})}")


def lands(v: str) -> bool:
    return v == LANDS


def not_lands(v: str) -> bool:
    return v != LANDS


def table(rows: list[tuple[str, str, str]], header: tuple[str, str, str]) -> list[str]:
    lines = [f"| {header[0]} | {header[1]} | {header[2]} |", "| --- | --- | --- |"]
    lines += [f"| {cell(a)} | {cell(b)} | {cell(c)} |" for a, b, c in rows]
    return lines


def by_variant(links: list[dict]) -> dict[str, list[dict]]:
    out: dict[str, list[dict]] = collections.defaultdict(list)
    for link in links:
        out[link["variant"]].append(link)
    return out


def github_section(title: str, intro: list[str], links: list[dict], observed: dict, targets: dict[str, str]) -> list[str]:
    results = {r["code"]: r for r in observed["results"]}
    lines = [f"### {title}", "", *intro, ""]
    for variant, variant_links in by_variant(links).items():
        lines += [f"#### Links to the {VARIANT_TITLE[variant]}: [{targets[variant]}]({targets[variant]})", ""]
        rows = []
        for link in variant_links:
            seen = results[link["code"]]
            rows.append((
                f"[{link['code']}]({link['href']}) `{link['form']}`",
                f"`{link['heading']}`",
                f"opened directly: {short(seen['direct']['verdict'])}; clicked in the source: {short(seen['click']['verdict'])}",
            ))
        lines += table(rows, ("Link (click it)", "Must land on", "Observed in Chromium")) + [""]
    return lines


def method_text(m: dict, o: dict, out: pathlib.Path) -> str:
    stamp = m["stamp"]
    gh = m["github"]
    board = m["board"]
    local = m["local"]
    vscode = m["vscode"]
    lines = [
        "This is the acceptance document: every link below is a real demonstration you can click, beside the "
        "heading it must land on and what a real Chromium did with it. Nothing here changed onetaskgraph's "
        f"library or CLI. Every item is from harness run `{stamp}`.",
        "",
        "**What was built.** Every target holds the same five headings, in this order: `## What`, `## Budgets`, "
        "`### Budget demo-quota-seconds`, `### Step 2.b: retry (once)`, and a second `## Budgets` at the end "
        "(the repeated heading). Each section starts with a bold line naming which of the five it is, followed by "
        "filler, so a landing is told from the top of the page at a glance. Each destination has three targets: "
        "plain, one with `<a id=\"<slug>\"></a>` on the line before each heading, and one with "
        "`<a name=\"<slug>\"></a>` there. A source links every heading of every target in every form:",
        "",
        "- `S`: `<target>#<slug>`, GitHub's heading slug (`what`, `budgets`, `budget-demo-quota-seconds`, "
        "`step-2b-retry-once`, and `budgets-1` for the repeat).",
        "- `U`: `<target>#user-content-<slug>`.",
        "- For local Markdown, also `A` and `B`: the same two fragments after the target's absolute path.",
        "",
        "A link's code is `<destination>-<target>-<form><heading>`: target `P` plain, `I` `<a id>`, `N` "
        "`<a name>`. So `GI-I-U2` is the GitHub-issue link to the `<a id>` target's `## Budgets` in the "
        "`#user-content-` form.",
        "",
        "**How it was observed.** `observe.py` drove Chromium "
        f"{o['github']['browser'].split()[-1]} through Playwright, at a 1280×800 viewport. A link *lands* when, "
        "after the page settles, the heading it names (by position, so the repeated `## Budgets` is told from "
        "the first) is at the top of the viewport. GitHub links were observed twice. *Opened directly* is a fresh "
        "tab loading the URL, which is what clicking the link in this report does. *Clicked in the source* loads "
        "the source issue and clicks the link there, which is what a reader of the design document does.",
        "",
        "**The one command.** On branch `" + BRANCH + "`, `scripts/spikes/section-links/run.sh` builds the whole set "
        "and prints every item's URL or path: GitHub issues, the board copy, Linear, the committed local pair and "
        "its upload, and the VS Code-style preview. It then observes every browser-observable link. "
        "`harness.py <step>` re-runs one destination; `report.py` writes these answers from a run.",
        "",
    ]

    # GitHub issues
    lines += github_section(
        "GitHub issue",
        [
            f"Three targets and a source in `nickderobertis/onetaskgraph-live-scratch`, created through `gh issue create`. "
            f"Source: [{gh['source']['url']}]({gh['source']['url']}).",
        ],
        gh["links"], o["github"], {v: t["url"] for v, t in gh["targets"].items()},
    )

    # Board copy
    tasks = board["tasks"]
    lines += github_section(
        "GitHub Projects copy (today's behaviour)",
        [
            f"A scratch local-md project held one target task per variant and one source document. The document "
            f"linked each target task by its local location (an absolute path) plus `#<slug>` or "
            f"`#user-content-<slug>`. The installed `{board['cli']}` then ran its real `project copy` and "
            f"`document copy` onto board {m.get('board_ref', '#1')}, with issues in the scratch repository. "
            f"The copied source document is [{board['document']['url']}]({board['document']['url']}). Each link "
            "below is the one that copy wrote. The copy rewrote the local path to the copied task's issue URL and "
            "carried the fragment through unchanged.",
        ],
        board["links"], o["board"], {v: t["url"] for v, t in tasks.items()},
    )

    # Linear
    lines += ["### Linear issue and Linear document", ""]
    if "linear" not in m:
        lines += [
            "**Pending: not built yet.** The TES2 scratch key (`LINEAR_API_KEY`, `LINEAR_WRITE_TEAM=TES2`) has not "
            "been supplied to this dispatch. The organization's production key is out of bounds and was not used. "
            "Once the key is set, `scripts/spikes/section-links/harness.py linear --out <dir>` builds the issue and "
            "document set in TES2 (`run.sh` builds it with everything else). It reads every body back, and this "
            "section is re-rendered with each link, the body Linear stored, and **awaiting your click** beside each.",
            "",
        ]
    else:
        for kind in ("issues", "documents"):
            part = m["linear"][kind]
            label = "issue" if kind == "issues" else "document"
            lines += [f"#### Linear {label}", "",
                      f"Source {label}: [{part['source']['url']}]({part['source']['url']}). Linear's web app needs a "
                      "logged-in session, which the dispatch does not hold, so **every result here awaits your click**.",
                      ""]
            for variant, variant_links in by_variant(part["links"]).items():
                target = part["targets"][variant]["url"]
                lines += [f"##### Links to the {VARIANT_TITLE[variant]}: [{target}]({target})", ""]
                rows = [(f"[{l['code']}]({l['href']}) `{l['form']}`", f"`{l['heading']}`",
                         "awaiting your click; Linear stored the link as " + (f"`{l['stored_href']}`" if l.get("stored_href") else "**no link at all**"))
                        for l in variant_links]
                lines += table(rows, ("Link (click it)", "Must land on", "Result")) + [""]
            stored = part["targets"]["anchor-id"].get("description") or part["targets"]["anchor-id"].get("content") or ""
            lines += ["What Linear stored for the opening of the `<a id>` target (sent as Markdown, read back through the API):", "", "```markdown", stored[:900], "```", ""]

    # Local: GitHub file view
    up = local["uploaded"]
    fv = {r["code"]: r for r in o["fileview"]["results"]}
    lines += ["### Local Markdown in GitHub's file view", "",
              f"The committed pair (`scripts/spikes/section-links/local/` on branch `{BRANCH}`), uploaded unchanged to "
              f"branch `{local['branch']}` of the scratch repository. The source is [source.md]({up['source.md']}). Each "
              "link was clicked in that rendered source. Forms `S` and `U` are listed as the URL GitHub resolved them to, "
              "so clicking one here is a direct load of the same address. Forms `A` and `B` resolve to a file that does not "
              f"exist, so they are listed as text: click them inside [source.md]({up['source.md']}) to see the 404.", ""]
    names = {"plain": "target.md", "anchor-id": "target-anchor-id.md", "anchor-name": "target-anchor-name.md"}
    for variant, variant_links in by_variant(local["links"]).items():
        lines += [f"#### Links to the {VARIANT_TITLE[variant]}: [{names[variant]}]({up[names[variant]]})", ""]
        rows = []
        for link in variant_links:
            seen = fv[link["code"]]["click"]
            if link["form"].startswith("<absolute path>"):
                shown = f"{link['code']} `{link['form']}` → `{seen.get('clicked_href', link['href'])}`"
            else:
                shown = f"[{link['code']}]({seen['clicked_href']}) `{link['form']}`"
            rows.append((shown, f"`{link['heading']}`", "clicked in the file view: " + short(seen["verdict"])))
        lines += table(rows, ("Link", "Must land on", "Observed in Chromium")) + [""]

    # Local: VS Code-style preview
    vs = {r["code"]: r for r in o["vscode"]["results"]}
    lines += ["### Local Markdown in a VS Code-style preview", "",
              "VS Code's preview cannot be driven headless, so `vscode-preview/render.mjs` reproduces it. Link "
              "resolution is VS Code's own code: `vscode-markdown-languageservice` 0.5.0, the version VS Code's "
              "`markdown-language-features` extension pins, asked `resolveLinkTarget` with the repository checkout "
              "as the workspace folder. The preview's click handling and fragment scroll are ported from that "
              "extension, and the page is rendered by markdown-it with `html: true` and VS Code's slugifier. The "
              "rendered preview was then clicked in Chromium. To confirm with your own eyes, open "
              f"`scripts/spikes/section-links/local/source.md` of branch `{BRANCH}` in VS Code, with the repository "
              "as the workspace, open its preview (Ctrl+Shift+V), and click each code. Each link below opens the "
              "committed target file at the address that link names.", ""]
    for variant, variant_links in by_variant(local["links"]).items():
        target = pathlib.Path(local["dir"], names[variant])
        lines += [f"#### Links to the {VARIANT_TITLE[variant]}: `{names[variant]}`", ""]
        rows = []
        for link in variant_links:
            frag = link["href"].split("#", 1)[1]
            rows.append((f"[{link['code']}]({target.as_uri()}#{frag}) `{link['href'] if not link['form'].startswith('<absolute') else link['form']}`",
                         f"`{link['heading']}`",
                         "clicked in the preview: " + short(vs[link["code"]]["click"]["verdict"]) +
                         ("" if not vscode["resolutions"].get(link["code"]) else
                          f" (language service: {vscode['resolutions'][link['code']]['preview'].split(',')[0].split(':')[0]})")))
        lines += table(rows, ("Link", "Must land on", "Observed in Chromium")) + [""]
    return "\n".join(lines).rstrip() + "\n"


def kept_summary(o: dict) -> str:
    """How the in-app clicks whose fragment names an element compare with the source offset."""
    diffs = [abs(r["click"]["scrollY"] - r["click"]["sourceScrollY"])
             for surface in ("github", "board") for r in o[surface]["results"]
             if r["form"] == "#user-content-<slug>" and r["variant"] != "plain" and "sourceScrollY" in r["click"]]
    if len(diffs) != 20:
        sys.exit(f"expected 20 measured in-app clicks with a named element, got {len(diffs)}")
    equal = sum(1 for d in diffs if d == 0)
    off = sorted(d for d in diffs if d)
    return f"{equal} of {len(diffs)} such clicks equal to it" + (f", the rest within {off[-1]} px of it" if off else "")


def findings(m: dict, o: dict) -> list[str]:
    gd, gc = group(o["github"]["results"], "direct"), group(o["github"]["results"], "click")
    bd, bc = group(o["board"]["results"], "direct"), group(o["board"]["results"], "click")
    fv = group(o["fileview"]["results"], "click")
    vs = group(o["vscode"]["results"], "click")
    gh = m["github"]
    t = {v: x["url"] for v, x in gh["targets"].items()}
    bt = {v: x["url"] for v, x in m["board"]["tasks"].items()}
    out = []

    for g in (gd, bd):
        only(g, "anchor-id", "#user-content-<slug>", lands, "the <a id> target with #user-content- lands when opened directly")
        for variant, form in (("plain", "#<slug>"), ("plain", "#user-content-<slug>"), ("anchor-id", "#<slug>"),
                              ("anchor-name", "#<slug>"), ("anchor-name", "#user-content-<slug>")):
            only(g, variant, form, not_lands, f"{variant} {form} does not land on an issue")
    for g in (gc, bc):
        for key in g:
            only(g, *key, not_lands, "no link clicked inside an issue lands")
    html_id = gh["targets"]["anchor-id"]["body_html"]
    html_name = gh["targets"]["anchor-name"]["body_html"]
    html_plain = gh["targets"]["plain"]["body_html"]
    if 'id="user-content-what"' not in html_id or 'name="user-content-what"' not in html_name or "<h2 dir=\"auto\">What</h2>" not in html_plain or "id=" in html_plain.split("What</h2>")[0].split("<h2")[-1]:
        sys.exit("the stored body_html does not back the sanitizer finding")
    out.append(
        "**GitHub issue: exactly one form lands, and only when the link is opened rather than clicked inside GitHub.** "
        "An issue renders its headings with no `id` at all. GitHub's API `body_html` gives "
        f"`<h2 dir=\"auto\">What</h2>` ([plain target]({t['plain']})), and no element of the live page carries a heading id. "
        "So neither `#<slug>` nor `#user-content-<slug>` can land on a plain issue, and the target record **must carry "
        "anchor markup** to be addressable. The sanitizer keeps both raw anchors and prefixes each value with "
        "`user-content-`: `<a id=\"what\">` is stored as `<a id=\"user-content-what\">` "
        f"([target]({t['anchor-id']})), and `<a name=\"what\">` as `<a name=\"user-content-what\">` "
        f"([target]({t['anchor-name']})). Of all the combinations, only **`<a id=\"<slug>\">` in the target plus a "
        f"`#user-content-<slug>` link** lands. Each of its five links lands, the repeated heading included "
        f"([GI-I-U5]({t['anchor-id']}#user-content-budgets-1)). The `<a name>` anchor does not land in either form, "
        "because the issue body is rendered by script after load, and GitHub then scrolls only to an element whose "
        "`id` is the fragment. A plain `#<slug>` lands on nothing, even beside an `<a id>`, because GitHub does not "
        "map it to the prefixed id on issues (it does in the file view)."
    )
    out.append(
        "**GitHub issue: a link clicked inside GitHub never lands, whatever its form.** Clicking a link from one issue "
        "to another is GitHub's in-app navigation. It never scrolls to the fragment. Where the fragment names an "
        "element of the target (the `#user-content-` forms against the anchored targets), the page keeps the source "
        "issue's scroll offset: " + kept_summary(o) + ", none with the heading at the top. "
        "Every other click opens the target at its top. `probe_click.py` showed no wait (up to 12 s) and no reload "
        "moving the kept offset. "
        "So the working form lands for a reader who opens the link in a new tab, pastes it or follows it from outside "
        "GitHub (a Linear page, a local file, this report), but not one who clicks it inside another GitHub issue. "
        "That is GitHub's behaviour, not something a link's form can fix, and the plan should state it rather than "
        "promise a landing."
    )
    only(bd, "anchor-id", "#user-content-<slug>", lands, "board copy direct")
    first = m["board"]["links"][0]
    if not first["href"].startswith(bt["plain"] + "#") or "#" not in first["local_href"]:
        sys.exit("the board copy did not carry the fragment as the finding states")
    for variant, marker in (("anchor-id", '<a id="what"></a>'), ("anchor-name", '<a name="what"></a>')):
        number = bt[variant].rstrip("/").rsplit("/", 1)[-1]
        body = subprocess.run(["gh", "api", f"repos/nickderobertis/onetaskgraph-live-scratch/issues/{number}", "--jq", ".body"],
                              capture_output=True, text=True, check=True).stdout
        if marker not in body:
            sys.exit(f"the board task {bt[variant]} does not hold {marker} as the finding states")
    out.append(
        "**GitHub Projects copy: today's copy already carries the fragment unchanged.** "
        f"`{m['board']['cli']} document copy` rewrote each link of the source document from the target task's local path "
        f"(`{first['local_href'].rsplit('/', 1)[-1]}`) to the copied task's issue URL and kept `#<fragment>` byte for byte, "
        f"for example `[GP-P-S1]({first['href']})` in [the copied document]({m['board']['document']['url']}). The copy "
        "also carries the task body's raw `<a id>` and `<a name>` lines to the board unchanged. So on the board the "
        "result is exactly the GitHub-issue result: the `<a id>` target task plus a `#user-content-<slug>` link lands "
        "when opened, and nothing else does. A link authored today as `#<slug>` (what a local viewer expects) "
        "therefore arrives on the board in a form that lands nowhere. Landing needs the target to carry "
        "`<a id=\"<slug>\"></a>`, and the copy to rewrite the fragment to `#user-content-<slug>`, or the source to be "
        "written with it."
    )
    for key in fv:
        variant, form = key
        if form.startswith("<absolute"):
            only(fv, variant, form, lambda v: "File not found" in v, "absolute paths 404 in the file view")
        else:
            only(fv, variant, form, lands, "relative links land in the file view")
    out.append(
        "**Local Markdown in GitHub's file view: every relative link lands, in both forms, on every target, anchor or "
        f"not.** GitHub renders a Markdown file's headings with its own `a#user-content-<slug>` anchor. Its file view "
        "also maps a bare `#<slug>` onto it, so `./target.md#<slug>` and `./target.md#user-content-<slug>` both land, "
        f"the repeat included ([source]({m['local']['uploaded']['source.md']})). An absolute path never resolves. "
        "GitHub reads a leading `/` as the repository root, so the machine path becomes "
        "`blob/<branch>/home/nick/…/target.md`, which answers *File not found*."
    )
    for key in vs:
        variant, form = key
        if form.startswith("<absolute"):
            only(vs, variant, form, lambda v: v.startswith("opens nothing"), "absolute paths open nothing in VS Code")
        elif form == "#<slug>":
            only(vs, variant, form, lands, "#<slug> lands in VS Code")
        else:
            only(vs, variant, form, not_lands, "#user-content- does not land in VS Code")
    out.append(
        "**Local Markdown in a VS Code-style preview: only `./target.md#<slug>` lands.** VS Code's language service "
        "matches a fragment against its own heading slugs (GitHub's algorithm, including `-1` for a repeat) and against "
        "raw HTML ids, so `#<slug>` lands on all three targets. `#user-content-<slug>` matches nothing, so the preview "
        "opens the target at its top. VS Code also reads a leading `/` as **workspace-relative**: an absolute machine "
        "path resolves to `<workspace>/home/nick/…/target.md`, which does not exist, and with no folder open it "
        "resolves to nothing (`resolveInternalDocumentLink` in `vscode-markdown-languageservice` 0.5.0)."
    )
    out.append(
        "**No single form lands everywhere.** In a GitHub issue (and so on the board) only `#user-content-<slug>`, "
        "against a target carrying `<a id=\"<slug>\"></a>`, lands. In the VS Code preview only `#<slug>` lands. In "
        "GitHub's file view both land. Every local viewer needs a relative path. So the contract has to differ by "
        "destination. The target record carries `<a id=\"<slug>\"></a>` before each linked heading wherever it lands "
        "(harmless locally: VS Code and the file view resolve it too). Locally authored links are relative paths with "
        "`#<slug>`. A copy to a GitHub destination rewrites each such fragment to `#user-content-<slug>`. A slug that "
        "is not unique within its record must take the `-1`, `-2` suffix, and both viewers agree on that suffix."
    )
    if "linear" not in m:
        out.append(
            "**Linear: pending, not yet established.** The TES2 scratch key has not been supplied, so no Linear issue "
            "or document has been built and nothing is yet known about what Linear stores for a fragment or a raw anchor. "
            "The one documented claim found is a third-party tutorial "
            "(https://www.guideflow.com/et-ee/tutorial/how-to-copy-a-link-to-header-on-linear), which says a heading "
            "shows a link icon on hover that copies \"the header's URL\". It gives no URL shape. Linear's own editor "
            "documentation (https://linear.app/docs/editor) says nothing about heading links or HTML anchors, and no "
            "changelog entry from late August to October 22, 2026 mentions one. Whether a Linear link lands is the "
            "question the user answers by clicking, once the set is built."
        )
    else:
        out.extend(linear_findings(m))
    return out


def linear_findings(m: dict) -> list[str]:
    out = []
    for kind in ("issues", "documents"):
        part = m["linear"][kind]
        label = "issue" if kind == "issues" else "document"
        kept = collections.Counter()
        for link in part["links"]:
            stored = link.get("stored_href")
            kept["fragment kept" if stored and stored == link["href"] else ("changed" if stored else "dropped")] += 1
        id_body = part["targets"]["anchor-id"].get("description") or part["targets"]["anchor-id"].get("content") or ""
        name_body = part["targets"]["anchor-name"].get("description") or part["targets"]["anchor-name"].get("content") or ""
        out.append(
            f"**Linear {label}: what Linear stored.** Of {len(part['links'])} links sent, "
            + ", ".join(f"{n} {k}" for k, n in sorted(kept.items()))
            + f". The raw `<a id>` anchor {'survived' if '<a id=' in id_body else 'did not survive'} and the raw "
            f"`<a name>` anchor {'survived' if '<a name=' in name_body else 'did not survive'} the round trip "
            f"([`<a id>` target]({part['targets']['anchor-id']['url']}), [`<a name>` target]({part['targets']['anchor-name']['url']})). "
            "Whether a stored link lands is left open, as the question the user answers by clicking the links listed above."
        )
    return out


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    out = pathlib.Path(args.out).resolve()
    m = json.loads((out / "manifest.json").read_text())
    o = json.loads((out / "observations.json").read_text())
    answers = {
        "spike": "spike-section-links",
        "branch": BRANCH,
        "harness": (
            f"`scripts/spikes/section-links/` on branch `{BRANCH}`. One command builds the whole demonstration set, "
            "prints every item's URL or path and observes every browser-observable link: "
            "`scripts/spikes/section-links/run.sh [--out DIR]`. It needs `gh`, `GH_PROJECTS_TOKEN`, `GH_PROJECTS_OWNER`, "
            "`GH_PROJECTS_NUMBER`, `LINEAR_API_KEY`, `LINEAR_WRITE_TEAM=TES2`, node, uv, a Playwright Chromium and the "
            "installed `onetaskgraph`. `harness.py <github|board|linear|local|vscode> --out DIR` re-runs one destination, "
            "`observe.py --out DIR` observes it, and `report.py --out DIR` writes this report's answers from the run."
        ),
        "method": method_text(m, o, out),
        "candidates": [],
        "findings": findings(m, o),
    }
    (out / "answers.json").write_text(json.dumps(answers, indent=2) + "\n")
    print(f"answers: {out / 'answers.json'}")


if __name__ == "__main__":
    main()
