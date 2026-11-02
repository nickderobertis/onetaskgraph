#!/usr/bin/env python3
"""Section-links spike: build every demonstration of a link to a heading, on every destination.

Throwaway harness for the spike `spike-section-links` of the plan `authoring:section-links`.
It changes nothing in onetaskgraph. One run creates, on each destination, three targets that
hold the same five headings (plain; each heading preceded by `<a id="<slug>"></a>`; each
preceded by `<a name="<slug>"></a>`) and a source linking every heading of every target in
every candidate form, then prints every item's URL or path and writes `manifest.json` into
the output directory for `observe.py` and the report.

Destinations, each a subcommand so a person can re-run one:
  github   issues in nickderobertis/onetaskgraph-live-scratch, through `gh`
  board    a scratch local-md project copied by the installed CLI's real `project copy` and
           `document copy` onto board #1 (GH_PROJECTS_OWNER / GH_PROJECTS_NUMBER)
  linear   issues and documents in team TES2 (LINEAR_WRITE_TEAM), through Linear's GraphQL API
  local    the committed Markdown under local/, and its upload to the scratch repository
  vscode   a VS Code-style preview of local/, rendered by vscode-preview/render.mjs
  all      every one of the above, in that order

Writes go only to the scratch repository, board #1 and the Linear team named by
LINEAR_WRITE_TEAM, which must be TES2. No title begins with the live janitor's cleanup prefix.
"""

from __future__ import annotations

import argparse
import base64
import datetime
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.request

HERE = pathlib.Path(__file__).resolve().parent
LOCAL = HERE / "local"
REPO = "nickderobertis/onetaskgraph-live-scratch"
FILES_BRANCH = "section-links-spike"
LINEAR_TEAM = "TES2"
JANITOR_PREFIX = "onetaskgraph live cleanup "

# The five heading kinds the task names: two plain h2s, an h3 with a hyphenated id, an h3
# with punctuation, and an h2 whose text repeats an earlier one.
HEADINGS = [
    (2, "What"),
    (2, "Budgets"),
    (3, "Budget demo-quota-seconds"),
    (3, "Step 2.b: retry (once)"),
    (2, "Budgets"),
]

VARIANTS = {
    "P": ("plain", "no anchor markup"),
    "I": ("anchor-id", 'each heading preceded by `<a id="<slug>"></a>`'),
    "N": ("anchor-name", 'each heading preceded by `<a name="<slug>"></a>`'),
}

FORMS = {
    "S": "#<slug>",
    "U": "#user-content-<slug>",
}


def slugs() -> list[str]:
    """GitHub's heading slug, as the task states it, with a repeat suffixed `-1`, `-2`..."""
    seen: dict[str, int] = {}
    out = []
    for _, text in HEADINGS:
        base = re.sub(r"[^\w\- ]", "", text.lower(), flags=re.UNICODE).replace(" ", "-")
        if base in seen:
            seen[base] += 1
            out.append(f"{base}-{seen[base]}")
        else:
            seen[base] = 0
            out.append(base)
    return out


def heading_label(index: int) -> str:
    level, text = HEADINGS[index]
    label = f"{'#' * level} {text}"
    if index == 4:
        label += " (the second heading with this text, at the end)"
    return label


def fragment(form: str, slug: str) -> str:
    """`S` and `A` are `#<slug>`; `U` and `B` are `#user-content-<slug>`."""
    return f"user-content-{slug}" if form in ("U", "B") else slug


def target_body(variant: str, intro: str) -> str:
    lines = [
        f"**Section-links spike target — {VARIANTS[variant][0]}** ({VARIANTS[variant][1]}). {intro}",
        "",
        "Each section is padded so a link that lands on its heading visibly scrolls the page; "
        "the first line under each heading names the section you landed on.",
        "",
    ]
    for index, ((level, text), slug) in enumerate(zip(HEADINGS, slugs())):
        if variant == "I":
            lines.append(f'<a id="{slug}"></a>')
        elif variant == "N":
            lines.append(f'<a name="{slug}"></a>')
        lines.append(f"{'#' * level} {text}")
        lines.append("")
        repeat = " — the second heading with this text" if index == 4 else ""
        lines.append(f"**You are in section {index + 1} of 5: `{'#' * level} {text}`{repeat}.**")
        lines.append("")
        for n in range(1, 15):
            lines.append(
                f"Filler paragraph {n} of section {index + 1} ({text}). It exists only to push "
                "the next heading well below the fold, so that a link landing on a heading "
                "and a link landing at the top of the page are told apart at a glance."
            )
            lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def link_code(dest: str, variant: str, form: str, index: int) -> str:
    return f"{dest}-{variant}-{form}{index + 1}"


def source_section(dest: str, variant: str, target_ref: str, target_display: str, extra: list[tuple[str, str, str]] = ()) -> tuple[list[str], list[dict]]:
    """One section of a source: every heading of one target, in every form.

    The forms are `S` and `U` against `target_ref`, then each `(code, label, ref)` of `extra`.
    """
    lines = [f"## Links to the {VARIANTS[variant][0]} target", "", f"Target: {target_display}", ""]
    links = []
    forms = [(code, label, target_ref) for code, label in FORMS.items()] + list(extra)
    for form, form_label, ref in forms:
        lines.append(f"Form `{form_label}`:")
        lines.append("")
        for index, slug in enumerate(slugs()):
            code = link_code(dest, variant, form, index)
            href = f"{ref}#{fragment(form, slug)}"
            lines.append(f"- [{code} {HEADINGS[index][1]}]({href}) — must land on `{heading_label(index)}`")
            links.append(
                {
                    "code": code,
                    "variant": VARIANTS[variant][0],
                    "form": form_label,
                    "heading_index": index,
                    "heading": heading_label(index),
                    "href": href,
                }
            )
        lines.append("")
    return lines, links


def source_body(intro: str, sections: list[tuple[list[str], list[dict]]]) -> tuple[str, list[dict]]:
    lines = [
        f"**Section-links spike source.** {intro}",
        "",
        "Every link is labelled `<destination>-<target>-<form><heading>`: target `P` plain, `I` "
        "`<a id>` anchors, `N` `<a name>` anchors; form `S` `#<slug>`, `U` `#user-content-<slug>` "
        "(`A`/`B` the same two against an absolute path). Each names the heading it must land on.",
        "",
    ]
    links: list[dict] = []
    for section_lines, section_links in sections:
        lines.extend(section_lines)
        links.extend(section_links)
    return "\n".join(lines).rstrip() + "\n", links


# --------------------------------------------------------------------------------------------
# Plumbing


def run(cmd: list[str], *, env=None, cwd=None, input_text=None, check=True) -> str:
    proc = subprocess.run(cmd, env=env, cwd=cwd, input=input_text, capture_output=True, text=True)
    if check and proc.returncode != 0:
        sys.exit(f"command failed ({proc.returncode}): {' '.join(cmd)}\n{proc.stderr.strip()}\n{proc.stdout.strip()}")
    return proc.stdout


def gh(*args: str, input_text=None) -> str:
    return run(["gh", *args], input_text=input_text)


def check_title(title: str) -> str:
    if title.startswith(JANITOR_PREFIX):
        sys.exit(f"refusing a title the live janitor would claim: {title!r}")
    return title


class Manifest:
    def __init__(self, out: pathlib.Path):
        self.path = out / "manifest.json"
        self.data = json.loads(self.path.read_text()) if self.path.exists() else {}

    def put(self, key: str, value) -> None:
        self.data[key] = value
        self.path.write_text(json.dumps(self.data, indent=2) + "\n")


def say(label: str, where: str) -> None:
    print(f"  {label:<44} {where}")


# --------------------------------------------------------------------------------------------
# GitHub issues


def github_issue(title: str, body: str, scratch: pathlib.Path) -> dict:
    body_file = scratch / f"body-{re.sub(r'[^a-z0-9]+', '-', title.lower())}.md"
    body_file.write_text(body)
    url = gh("issue", "create", "--repo", REPO, "--title", check_title(title), "--body-file", str(body_file)).strip().splitlines()[-1]
    number = url.rstrip("/").rsplit("/", 1)[-1]
    rendered = json.loads(gh("api", f"repos/{REPO}/issues/{number}", "-H", "Accept: application/vnd.github.full+json"))
    return {"url": url, "number": int(number), "body": rendered["body"], "body_html": rendered["body_html"]}


def build_github(m: Manifest, stamp: str, scratch: pathlib.Path) -> None:
    print("GitHub issues (" + REPO + "):")
    targets = {}
    for variant in VARIANTS:
        title = f"Section-links spike {stamp}: target ({VARIANTS[variant][0]})"
        targets[variant] = github_issue(title, target_body(variant, f"Run `{stamp}`."), scratch)
        say(f"target {VARIANTS[variant][0]}", targets[variant]["url"])
    sections = [source_section("GI", v, targets[v]["url"], targets[v]["url"]) for v in VARIANTS]
    body, links = source_body(f"Run `{stamp}`. Links to the three target issues of this run.", sections)
    source = github_issue(f"Section-links spike {stamp}: source", body, scratch)
    say("source", source["url"])
    m.put("github", {"targets": {VARIANTS[v][0]: t for v, t in targets.items()}, "source": source, "links": links})


# --------------------------------------------------------------------------------------------
# GitHub Projects copy through the installed CLI


def ot_env(scratch: pathlib.Path) -> dict:
    env = dict(os.environ)
    # Hold the CLI to the scratch configuration alone: no user layer, no host secrets file.
    env["XDG_CONFIG_HOME"] = str(scratch / "xdg-config")
    env["XDG_CACHE_HOME"] = str(scratch / "xdg-cache")
    for key in list(env):
        if key.startswith("ONETASKGRAPH_"):
            del env[key]
    env["ONETASKGRAPH_INTERACTIVE"] = "false"
    return env


def only_item(answer: str) -> tuple[str, dict]:
    """The one item of a `show`/`create --json` answer: its qualified id and the item."""
    items = json.loads(answer)["items"]
    if len(items) != 1:
        sys.exit(f"expected one item, got {len(items)}")
    return items[0]["id"], items[0]["item"]


def copy_link(item: dict, destination: str) -> str:
    link = (item.get("metadata") or {}).get("onetaskgraph.copies", {}).get(destination)
    if not link:
        sys.exit(f"{item.get('id')} records no copy link to {destination}")
    return link


def build_board(m: Manifest, stamp: str, scratch: pathlib.Path, onetaskgraph: str) -> None:
    owner = os.environ["GH_PROJECTS_OWNER"]
    number = os.environ["GH_PROJECTS_NUMBER"]
    work = scratch / "board-copy"
    root = work / "local-md"
    (scratch / "xdg-config").mkdir(parents=True, exist_ok=True)
    root.mkdir(parents=True, exist_ok=True)
    (work / "onetaskgraph.yaml").write_text(
        "# Scratch configuration of the section-links spike: one local-md source, and board #1.\n"
        "sources:\n"
        "  spike-local:\n"
        "    plugin: local-md\n"
        "    config:\n"
        f"      root: {root}\n"
        "  spike-board:\n"
        "    plugin: github-projects\n"
        "    config:\n"
        f"      owner: {owner}\n"
        f"      project_number: {number}\n"
        f"      repository: {REPO}\n"
        "      token_env: GH_PROJECTS_TOKEN\n"
    )
    env = ot_env(scratch)

    def ot(*args: str) -> str:
        return run([onetaskgraph, *args], env=env, cwd=work)

    version = ot("--version").strip()
    settings = json.loads(ot("config", "show", "--json"))["settings"]
    names = sorted({entry["key"].split(".")[1] for entry in settings if entry["key"].startswith("sources.")})
    if names != ["spike-board", "spike-local"]:
        sys.exit(f"the scratch configuration resolved other sources too: {names}")
    print(f"GitHub Projects copy ({version}, board {owner}#{number}):")
    project_id = f"section-links-demo-{stamp}"
    ot("project", "create", "spike-local", "--id", project_id, "--title", check_title(f"Section-links spike {stamp}: demonstration project"))
    tasks = {}
    for variant in VARIANTS:
        body_file = work / f"task-{variant}.md"
        body_file.write_text(target_body(variant, f"Run `{stamp}`. Written to a local-md task, then copied by `onetaskgraph project copy`."))
        task_id, task = only_item(ot("task", "create", "spike-local", "--project", project_id, "--title",
                                     check_title(f"Section-links spike {stamp}: target task ({VARIANTS[variant][0]})"),
                                     "--body-file", str(body_file), "--json"))
        tasks[variant] = {"id": task_id, "location": task["location"]["path"]}
        say(f"local target task {VARIANTS[variant][0]}", tasks[variant]["location"])
    sections = [source_section("GP", v, tasks[v]["location"], f"`{tasks[v]['location']}`") for v in VARIANTS]
    body, links = source_body(f"Run `{stamp}`. Written to a local-md document linking each target task by its local location, then copied by `onetaskgraph document copy`.", sections)
    doc_file = work / "document.md"
    doc_file.write_text(body)
    doc_id = f"section-links-demo-{stamp}-source"
    ot("document", "create", "spike-local", "--project", project_id, "--id", doc_id, "--title",
       check_title(f"Section-links spike {stamp}: source document"), "--body-file", str(doc_file))
    doc_location = only_item(ot("document", "show", f"spike-local:{doc_id}", "--json"))[1]["location"]["path"]
    say("local source document", doc_location)
    project_copy = json.loads(ot("project", "copy", f"spike-local:{project_id}", "--to", "spike-board", "--json"))
    document_copy = json.loads(ot("document", "copy", f"spike-local:{doc_id}", "--to", "spike-board", "--json"))
    copied_tasks = {}
    for variant, task in tasks.items():
        link = copy_link(only_item(ot("task", "show", task["id"], "--json"))[1], "spike-board")
        board_task = only_item(ot("task", "show", link, "--json"))[1]
        copied_tasks[VARIANTS[variant][0]] = {"local": task, "board_id": link, "url": board_task.get("url") or board_task["location"]["url"]}
        say(f"board target task {VARIANTS[variant][0]}", copied_tasks[VARIANTS[variant][0]]["url"])
    board_doc_id = copy_link(only_item(ot("document", "show", f"spike-local:{doc_id}", "--json"))[1], "spike-board")
    board_doc = only_item(ot("document", "show", board_doc_id, "--json"))[1]
    board_doc_url = board_doc.get("url") or board_doc["location"]["url"]
    say("board source document", board_doc_url)
    # What the copy wrote: each link of the copied document, as the board holds it.
    issue_number = board_doc_url.rstrip("/").rsplit("/", 1)[-1]
    stored = json.loads(gh("api", f"repos/{REPO}/issues/{issue_number}"))["body"]
    for link in links:
        found = re.search(r"\[" + re.escape(link["code"]) + r" [^\]]*\]\(([^)\s]+)\)", stored)
        link["local_href"] = link["href"]
        link["href"] = found.group(1) if found else None
    m.put("board", {
        "cli": version,
        "project_id": f"spike-local:{project_id}",
        "local_document": doc_location,
        "tasks": copied_tasks,
        "document": {"board_id": board_doc_id, "url": board_doc_url, "stored_body": stored},
        "project_copy": project_copy,
        "document_copy": document_copy,
        "links": links,
    })


# --------------------------------------------------------------------------------------------
# Linear


def linear(query: str, variables: dict) -> dict:
    request = urllib.request.Request(
        "https://api.linear.app/graphql",
        data=json.dumps({"query": query, "variables": variables}).encode(),
        headers={"Content-Type": "application/json", "Authorization": os.environ["LINEAR_API_KEY"]},
    )
    with urllib.request.urlopen(request, timeout=60) as response:
        answer = json.loads(response.read())
    if answer.get("errors"):
        sys.exit(f"Linear refused: {json.dumps(answer['errors'])}")
    return answer["data"]


ISSUE_FIELDS = "id identifier url title description"
DOC_FIELDS = "id slugId url title content"


def build_linear(m: Manifest, stamp: str) -> None:
    team_key = os.environ.get("LINEAR_WRITE_TEAM", "")
    if team_key != LINEAR_TEAM:
        sys.exit(f"LINEAR_WRITE_TEAM is {team_key!r}; this spike writes only to {LINEAR_TEAM}")
    teams = linear("query($key:String!){teams(filter:{key:{eq:$key}}){nodes{id key name organization{urlKey}}}}", {"key": team_key})["teams"]["nodes"]
    if len(teams) != 1:
        sys.exit(f"team {team_key} not found exactly once: {teams}")
    team = teams[0]
    print(f"Linear (team {team['key']}, workspace {team['organization']['urlKey']}):")

    def issue(title: str, body: str) -> dict:
        created = linear(f"mutation($i:IssueCreateInput!){{issueCreate(input:$i){{issue{{{ISSUE_FIELDS}}}}}}}",
                         {"i": {"teamId": team["id"], "title": check_title(title), "description": body}})["issueCreate"]["issue"]
        read = linear(f"query($id:String!){{issue(id:$id){{{ISSUE_FIELDS}}}}}", {"id": created["id"]})["issue"]
        return {**read, "sent": body}

    def document(title: str, body: str) -> dict:
        created = linear(f"mutation($i:DocumentCreateInput!){{documentCreate(input:$i){{document{{{DOC_FIELDS}}}}}}}",
                         {"i": {"teamId": team["id"], "title": check_title(title), "content": body}})["documentCreate"]["document"]
        read = linear(f"query($id:String!){{document(id:$id){{{DOC_FIELDS}}}}}", {"id": created["id"]})["document"]
        return {**read, "sent": body}

    out = {"team": team, "issues": {}, "documents": {}}
    for kind, make, dest in (("issue", issue, "LI"), ("document", document, "LD")):
        targets = {}
        for variant in VARIANTS:
            targets[variant] = make(f"Section-links spike {stamp}: target {kind} ({VARIANTS[variant][0]})", target_body(variant, f"Run `{stamp}`."))
            say(f"target {kind} {VARIANTS[variant][0]}", targets[variant]["url"])
        sections = [source_section(dest, v, targets[v]["url"], targets[v]["url"]) for v in VARIANTS]
        body, links = source_body(f"Run `{stamp}`. Links to the three target {kind}s of this run.", sections)
        source = make(f"Section-links spike {stamp}: source {kind}", body)
        say(f"source {kind}", source["url"])
        stored = source.get("description") if kind == "issue" else source.get("content")
        for link in links:
            found = re.search(r"\[" + re.escape(link["code"]) + r" [^\]]*\]\(<?([^)\s>]+)>?\)", stored or "")
            link["stored_href"] = found.group(1) if found else None
        out["issues" if kind == "issue" else "documents"] = {
            "targets": {VARIANTS[v][0]: t for v, t in targets.items()},
            "source": source,
            "links": links,
        }
    m.put("linear", out)


# --------------------------------------------------------------------------------------------
# Local Markdown, and its upload to the scratch repository


def write_local() -> list[dict]:
    LOCAL.mkdir(parents=True, exist_ok=True)
    names = {"P": "target.md", "I": "target-anchor-id.md", "N": "target-anchor-name.md"}
    for variant, name in names.items():
        (LOCAL / name).write_text(target_body(variant, "Committed by the section-links spike harness."))
    sections = []
    for variant, name in names.items():
        absolute = str(LOCAL / name)
        sections.append(source_section(
            "LM", variant, f"./{name}", f"`./{name}` relative to this file; forms A and B name it by its absolute path `{absolute}`",
            extra=[("A", "<absolute path>#<slug>", absolute), ("B", "<absolute path>#user-content-<slug>", absolute)],
        ))
    body, links = source_body("Committed by the section-links spike harness. Relative links name a file beside this one; absolute ones name it by its path on the machine that ran the harness.", sections)
    (LOCAL / "source.md").write_text(body)
    return links


def bootstrap_files_branch(stamp: str) -> None:
    """Create the spike's branch in the scratch repository, which may hold no commit at all.

    The scratch repository exists for the live lane's issues and is empty, and GitHub's
    contents and git-data APIs refuse an empty repository (409), so the branch is made by
    pushing one orphan commit holding only a note. Nothing else of the repository is touched.
    """
    with tempfile.TemporaryDirectory() as tmp:
        work = pathlib.Path(tmp)
        (work / "section-links-spike").mkdir()
        (work / "section-links-spike" / "README.md").write_text(
            "Demonstration files of onetaskgraph's section-links spike, uploaded by "
            "`scripts/spikes/section-links/harness.py`. One folder per run.\n")
        git = ["git", "-C", str(work), "-c", "user.name=section-links spike", "-c", "user.email=noreply@github.com",
               "-c", "credential.helper=", "-c", "credential.helper=!gh auth git-credential"]
        run([*git, "init", "-q", "-b", FILES_BRANCH])
        run([*git, "add", "-A"])
        run([*git, "commit", "-q", "-m", f"section-links spike {stamp}: branch for demonstration files"])
        run([*git, "push", "-q", f"https://github.com/{REPO}.git", f"HEAD:refs/heads/{FILES_BRANCH}"])


def build_local(m: Manifest, stamp: str) -> None:
    links = write_local()
    print("Local Markdown (committed):")
    for name in ("target.md", "target-anchor-id.md", "target-anchor-name.md", "source.md"):
        say(name, str(LOCAL / name))
    # The upload: the same four files on a branch of the scratch repository, one folder per run.
    exists = subprocess.run(["gh", "api", f"repos/{REPO}/git/ref/heads/{FILES_BRANCH}"], capture_output=True).returncode == 0
    if not exists:
        bootstrap_files_branch(stamp)
    folder = f"section-links-spike/{stamp}"
    uploaded = {}
    for name in ("target.md", "target-anchor-id.md", "target-anchor-name.md", "source.md"):
        content = base64.b64encode((LOCAL / name).read_bytes()).decode()
        gh("api", "-X", "PUT", f"repos/{REPO}/contents/{folder}/{name}", "-f", f"message=section-links spike {stamp}: {name}",
           "-f", f"content={content}", "-f", f"branch={FILES_BRANCH}")
        uploaded[name] = f"https://github.com/{REPO}/blob/{FILES_BRANCH}/{folder}/{name}"
    print("Local Markdown, uploaded to the scratch repository:")
    for name, url in uploaded.items():
        say(name, url)
    m.put("local", {"dir": str(LOCAL), "links": links, "uploaded": uploaded, "branch": FILES_BRANCH, "folder": folder})


def build_vscode(m: Manifest, out: pathlib.Path) -> None:
    node_dir = out / "vscode-preview"
    node_dir.mkdir(parents=True, exist_ok=True)
    for name in ("package.json", "render.mjs"):
        shutil.copy(HERE / "vscode-preview" / name, node_dir / name)
    if not (node_dir / "node_modules").exists():
        run(["npm", "install", "--no-audit", "--no-fund", "--silent"], cwd=node_dir)
    workspace = run(["git", "rev-parse", "--show-toplevel"], cwd=HERE).strip()
    html = node_dir / "html"
    run(["node", "render.mjs", "--workspace", workspace, "--dir", str(LOCAL), "--out", str(html)], cwd=node_dir)
    print("Local Markdown in a VS Code-style preview:")
    say("source preview", str(html / "source.html"))
    m.put("vscode", {"workspace": workspace, "html": str(html), "resolutions": json.loads((html / "resolutions.json").read_text())})


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("step", choices=["github", "board", "linear", "local", "vscode", "all"])
    parser.add_argument("--out", required=True, help="directory for manifest.json and scratch files (outside the repository)")
    parser.add_argument("--onetaskgraph", default=os.environ.get("ONETASKGRAPH", "onetaskgraph"), help="the installed CLI the board copy runs")
    parser.add_argument("--stamp", default=datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%d-%H%M%S"))
    args = parser.parse_args()
    out = pathlib.Path(args.out).resolve()
    scratch = out / "scratch"
    scratch.mkdir(parents=True, exist_ok=True)
    m = Manifest(out)
    m.put("stamp", m.data.get("stamp", args.stamp))
    stamp = m.data["stamp"]
    steps = ["github", "board", "linear", "local", "vscode"] if args.step == "all" else [args.step]
    for step in steps:
        if step == "github":
            build_github(m, stamp, scratch)
        elif step == "board":
            build_board(m, stamp, scratch, args.onetaskgraph)
        elif step == "linear":
            build_linear(m, stamp)
        elif step == "local":
            build_local(m, stamp)
        elif step == "vscode":
            build_vscode(m, out)
    print(f"manifest: {m.path}")


if __name__ == "__main__":
    main()
