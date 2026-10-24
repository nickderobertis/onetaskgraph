"""The spike's generated plan: seeded pseudo-random prose, never a real plan's content.

One project, eight tasks that are its members with ten blocked-by edges among them, and the
answers a `design-doc` rendering of its one document is made from, with a seeded-noise PNG
the document references. The bodies are generated to the sizes the brief states, and they
are written straight into a `local-md` folder; the document itself is rendered by the
store from the answers (see `run.py`), because approval reads its template provenance.

`python3 workload.py --root DIR --plan ID --out DIR [--variant N]` writes the folder and
the document's answers. `--variant N` with `--rewrite` instead replaces only the content of
every record already in the folder, keeping each front matter (and with it every link a
copy recorded), which is the "every body changed" re-copy.
"""

from __future__ import annotations

import argparse
import json
import random
import struct
import zlib
from pathlib import Path

WORDS = [
    "board",
    "store",
    "plan",
    "task",
    "project",
    "document",
    "copy",
    "source",
    "destination",
    "request",
    "page",
    "cursor",
    "token",
    "mutation",
    "query",
    "read",
    "write",
    "field",
    "option",
    "status",
    "label",
    "edge",
    "member",
    "item",
    "record",
    "budget",
    "limit",
    "quota",
    "window",
    "bucket",
    "reset",
    "point",
    "cost",
    "latency",
    "burst",
    "pacing",
    "interval",
    "retry",
    "backoff",
    "wait",
    "refusal",
    "answer",
    "response",
    "header",
    "schema",
    "contract",
    "version",
    "digest",
    "template",
    "rendering",
    "answer",
    "variable",
    "loader",
    "provenance",
    "asset",
    "image",
    "upload",
    "verify",
    "engine",
    "plugin",
    "trait",
    "capability",
    "predicate",
    "filter",
    "search",
    "walk",
    "scan",
    "orphan",
    "reference",
    "location",
    "path",
    "link",
    "origin",
    "counterpart",
    "journal",
    "undo",
    "repair",
    "land",
    "phase",
    "span",
    "timing",
    "measure",
    "figure",
    "table",
    "report",
    "finding",
    "candidate",
    "workload",
    "scenario",
    "harness",
    "proxy",
    "log",
    "the",
    "a",
    "an",
    "of",
    "to",
    "in",
    "on",
    "for",
    "with",
    "by",
    "from",
    "at",
    "as",
    "is",
    "are",
    "was",
    "be",
    "that",
    "this",
    "which",
    "it",
    "each",
    "every",
    "one",
    "two",
    "three",
    "no",
    "not",
    "only",
    "once",
    "twice",
    "more",
    "less",
    "than",
    "before",
    "after",
    "while",
    "when",
    "then",
    "so",
    "because",
    "but",
    "and",
    "or",
    "if",
    "whether",
    "where",
    "what",
    "who",
    "how",
    "why",
    "rather",
    "instead",
    "still",
    "reads",
    "writes",
    "sends",
    "keeps",
    "holds",
    "names",
    "carries",
    "returns",
    "refuses",
    "waits",
    "spends",
    "takes",
    "makes",
    "leaves",
    "finds",
    "counts",
    "orders",
    "compares",
    "records",
    "reports",
    "measures",
    "decides",
    "changes",
]

# Every record names the test board's scratch repository: the GitHub Projects plugin files an
# item's issue in the repository the item names, ahead of the one its source configures.
REPOSITORY = "github.com/nickderobertis/onetaskgraph-live-scratch"

TASK_EDGES = (  # (task, the task it is blocked by), ten edges among eight tasks
    (2, 1),
    (3, 1),
    (4, 2),
    (5, 2),
    (5, 3),
    (6, 4),
    (7, 5),
    (7, 6),
    (8, 6),
    (8, 7),
)

TASK_BYTES = (45_000, 45_600, 46_200, 46_800, 47_400, 48_000, 48_600, 49_000)
PROJECT_BYTES = 50_000
# The document's rendered body is about 34 KB and its local file, which also keeps the
# answers it was rendered from, about 71 KB. By the manager's ruling (2026-10-18) the brief's
# "about 77 KB" is that real document's *file*; the crozier design document this models sends
# a 28.5 KB body. A literal 77 KB body is representable — GitHub took 77,000 bytes in 63,500
# characters and refused 66,000 characters (`probe-body-limit`) — but was ruled out.
DOCUMENT_PROSE_BYTES = 41_500
# What `rendered_sizes` holds a generated folder to: each body's UTF-8 bytes, low and high,
# and the most characters it may have (GitHub's issue-body limit is 65,536 characters).
SIZES = {
    "project": (49_000, 52_000, 64_000),
    "task": (44_500, 50_000, 64_000),
    "document": (33_000, 38_000, 64_000),
}


def sentence(rng: random.Random) -> str:
    words = [rng.choice(WORDS) for _ in range(rng.randint(8, 20))]
    return " ".join(words).capitalize() + "."


def paragraph(rng: random.Random) -> str:
    text = " ".join(sentence(rng) for _ in range(rng.randint(3, 6)))
    lines, line = [], ""
    for word in text.split():
        if line and len(line) + 1 + len(word) > 90:
            lines.append(line)
            line = word
        else:
            line = f"{line} {word}" if line else word
    lines.append(line)
    return "\n".join(lines)


def prose(rng: random.Random, size: int, headings: tuple[str, ...] = ()) -> str:
    """Markdown of about `size` bytes, its paragraphs spread under `headings`."""
    parts: list[str] = []
    total = 0
    heading = 0
    while total < size:
        if headings and (
            not parts or (rng.random() < 0.08 and heading < len(headings))
        ):
            block = f"## {headings[min(heading, len(headings) - 1)]}"
            heading += 1
        elif rng.random() < 0.15:
            block = "\n".join(f"- {sentence(rng)}" for _ in range(rng.randint(2, 5)))
        else:
            block = paragraph(rng)
        parts.append(block)
        total += len(block.encode()) + 2
    return "\n\n".join(parts) + "\n"


def noise_png(seed: int, side: int = 186) -> bytes:
    """An incompressible RGB PNG of about 104 KB, seeded."""
    rng = random.Random(seed)
    raw = b"".join(b"\x00" + rng.randbytes(side * 3) for _ in range(side))

    def chunk(kind: bytes, data: bytes) -> bytes:
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    header = struct.pack(">IIBBBBB", side, side, 8, 2, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


def task_title(plan: str, number: int) -> str:
    return f"feat(spike): seeded task {number} of {plan}"


def task_content(plan: str, number: int, variant: int) -> str:
    rng = random.Random(f"{plan}/t{number}/{variant}")
    return f"# {task_title(plan, number)}\n\n" + prose(
        rng, TASK_BYTES[number - 1], ("What", "Why", "Acceptance criteria", "Notes")
    )


def project_content(plan: str, variant: int) -> str:
    rng = random.Random(f"{plan}/project/{variant}")
    return prose(rng, PROJECT_BYTES, ("Overview", "Workload", "Sizing", "Ten times"))


def front_matter(fields: dict) -> str:
    """Front matter in the shape a real plan file has: `metadata` a block mapping, one key a
    line, which is what lets the folder record a copy's link narrowly (a flow mapping is
    copied without it, and the next copy would find its counterpart another way)."""
    lines = ["---"]
    for key, value in fields.items():
        if key == "metadata":
            lines.append("metadata:")
            lines += [f"  {json.dumps(k)}: {json.dumps(v)}" for k, v in value.items()]
        else:
            lines.append(f"{key}: {json.dumps(value)}")
    return "\n".join(lines) + "\n---\n"


def write_plan(root: Path, plan: str, variant: int, bare_last: bool = False) -> None:
    """Write the folder. `bare_last` leaves the eighth task naming no repository, so the
    plan's tasks do not all name one repository and ai-orchestrator's approval resolves the
    design template without `--repository` (this host registers no checkout of the scratch
    repository); its issue still lands in the scratch repository, the board's own."""
    (root / "projects").mkdir(parents=True, exist_ok=True)
    (root / "tasks" / plan).mkdir(parents=True, exist_ok=True)
    (root / "documents").mkdir(parents=True, exist_ok=True)
    project = front_matter(
        {
            "title": f"Spike workload {plan}",
            "status": "todo",
            "metadata": {
                "onepipeline.schema_version": 3,
                "onepipeline.goal": {"text": plan},
            },
            "repositories": [REPOSITORY],
        }
    )
    (root / "projects" / f"{plan}.md").write_text(
        project + "\n" + project_content(plan, variant)
    )
    for number in range(1, 9):
        blocked_by = [f"{plan}/t{b}" for t, b in TASK_EDGES if t == number]
        fields = {
            "title": task_title(plan, number),
            "project": plan,
            "status": "todo",
            "metadata": {
                "onepipeline.id": f"t{number}",
                "onepipeline.persona": "implementer",
            },
            "repositories": [REPOSITORY],
        }
        if bare_last and number == 8:
            del fields["repositories"]
        if blocked_by:
            fields["depends_on"] = blocked_by
        text = front_matter(fields) + "\n" + task_content(plan, number, variant)
        (root / "tasks" / plan / f"t{number}.md").write_text(text)


def body_of(path: Path) -> str:
    """A record's rendered body: what follows its front matter, up to any stored answers."""
    text = path.read_text(encoding="utf-8")
    body = text.split("\n---\n", 1)[1]
    return body.split("<!-- onetaskgraph:template-answers", 1)[0]


def rendered_sizes(root: Path, plan: str) -> dict[str, tuple[int, int]]:
    """Every rendered body of the plan's folder, in UTF-8 bytes and characters, refusing a
    folder whose sizes are not the brief's (`SIZES`)."""
    paths = {"project": root / "projects" / f"{plan}.md"}
    paths |= {f"task t{n}": root / "tasks" / plan / f"t{n}.md" for n in range(1, 9)}
    paths["document"] = root / "documents" / f"{plan}-design.md"
    sizes = {}
    for label, path in paths.items():
        body = body_of(path)
        sizes[label] = (len(body.encode()), len(body))
        low, high, chars = SIZES[label.split(" ")[0]]
        if not low <= sizes[label][0] <= high or (chars and sizes[label][1] > chars):
            raise ValueError(
                f"{label} body is {sizes[label]} bytes/characters, not {low}-{high}"
            )
    return sizes


def design_answers(root: Path, plan: str, variant: int, image: str) -> dict:
    """The `design-doc` answers: seeded prose, and a planned-tasks table whose every row
    names its task by the absolute path the folder reports it at, as a real one does."""
    rng = random.Random(f"{plan}/document/{variant}")
    share = DOCUMENT_PROSE_BYTES // 8
    units = []
    for number in range(1, 4):
        units.append(
            {
                "name": f"Unit {number}",
                "repository": "onetaskgraph",
                "part": f"crates/unit-{number}",
                "summary": paragraph(rng).replace("\n", " "),
                "reversible": [
                    {"title": f"Choice {k}", "text": sentence(rng)} for k in range(1, 4)
                ],
                "decisions": [
                    {
                        "name": f"Decision {number}.{k}",
                        "justification": sentence(rng),
                        "content": [
                            {
                                "type": "prose",
                                "text": prose(rng, share // 2).strip(),
                            }
                        ],
                    }
                    for k in range(1, 3)
                ],
            }
        )
    location = (root / "tasks" / plan).resolve()
    depends = {t: [b for tt, b in TASK_EDGES if tt == t] for t in range(1, 9)}
    return {
        "what": prose(rng, share // 2).strip(),
        "why": prose(rng, share // 2).strip(),
        "architecture": prose(rng, share).strip(),
        "units": units,
        "acceptance_criteria": [sentence(rng) for _ in range(12)],
        "planned_tasks": [
            {
                "task": task_title(plan, t),
                "unit": f"Unit {1 + (t - 1) % 3}",
                "delivers": sentence(rng),
                "depends_on": ", ".join(task_title(plan, b) for b in depends[t])
                or "none",
                "location": f"{location}/t{t}.md",
            }
            for t in range(1, 9)
        ],
        "dag_image": image,
    }


def rewrite_bodies(root: Path, plan: str, variant: int) -> None:
    """Replace every task's and the project's content, keeping each front matter whole."""
    files = [(root / "projects" / f"{plan}.md", project_content(plan, variant))]
    files += [
        (root / "tasks" / plan / f"t{n}.md", task_content(plan, n, variant))
        for n in range(1, 9)
    ]
    for path, content in files:
        text = path.read_text()
        end = text.index("\n---\n", 4) + len("\n---\n")
        path.write_text(text[:end] + "\n" + content)


def main_args(
    root: Path,
    plan: str,
    out: Path,
    variant: int,
    rewrite: bool,
    bare_last: bool = False,
) -> None:
    out.mkdir(parents=True, exist_ok=True)
    if rewrite:
        rewrite_bodies(root, plan, variant)
    else:
        write_plan(root, plan, variant, bare_last)
        (out / "plan-dag.png").write_bytes(noise_png(20261018))
    answers = design_answers(root, plan, variant, "plan-dag.png")
    (out / "design-answers.json").write_text(json.dumps(answers, indent=1))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--plan", required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--variant", type=int, default=0)
    parser.add_argument("--rewrite", action="store_true")
    args = parser.parse_args()
    main_args(args.root, args.plan, args.out, args.variant, args.rewrite)


if __name__ == "__main__":
    main()
