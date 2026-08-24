"""Generate the Python contract and client surface from the running binary."""

from __future__ import annotations

import argparse
import json
import subprocess
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).parent
GENERATED = ROOT / "src" / "onetaskgraph_sdk" / "_generated"
RESPONSE_ROOTS = {
    "task_list": "QueryResponseOfQualifiedTask",
    "task_show": "QueryResponseOfQualifiedTask",
    "task_deps": "QueryResponseOfQualifiedEdge",
    "project_list": "QueryResponseOfQualifiedProject",
    "project_show": "QueryResponseOfQualifiedProject",
    "project_deps": "QueryResponseOfQualifiedEdge",
    "label_list": "QueryResponseOfQualifiedLabel",
    "search": "QueryResponseOfSearchHit",
    "sources_list": "SourceListing",
    "config_show": "EffectiveConfig",
}
RETURN_TYPES = {"sources_list": "list[SourceListing]"}


def binary(*args: str) -> str:
    """Run the workspace binary, building the exact artifact under generation."""
    result = subprocess.run(
        ["cargo", "run", "--quiet", "-p", "onetaskgraph", "--bin", "onetaskgraph", "--", *args],
        cwd=ROOT.parent.parent,
        check=True,
        text=True,
        capture_output=True,
    )
    return result.stdout


def leaves(prefix: tuple[str, ...] = ()) -> list[tuple[str, ...]]:
    """Discover public command leaves recursively from clap's emitted help."""
    help_text = binary(*prefix, "--help")
    in_commands = False
    commands: list[str] = []
    for line in help_text.splitlines():
        if line == "Commands:":
            in_commands = True
            continue
        if in_commands and line and not line.startswith(" "):
            break
        if in_commands and line.startswith("  "):
            name = line.strip().split()[0]
            if name not in {"help", "schema"}:
                commands.append(name)
    found: list[tuple[str, ...]] = []
    for command in commands:
        child = (*prefix, command)
        child_help = binary(*child, "--help")
        if "Commands:\n" in child_help:
            found.extend(leaves(child))
        else:
            found.append(child)
    return found


def generate_models(bundle: dict[str, Any], destination: Path) -> None:
    """Generate Pydantic models directly from every response schema in the bundle."""
    destination.mkdir(parents=True, exist_ok=True)
    exports: list[str] = []
    for root in sorted(set(RESPONSE_ROOTS.values()) | {"SourceFailure", "QueryPlan"}):
        schema = bundle["roots"][root]
        module = camel_to_snake(root)
        source = destination / f"{module}.py"
        with tempfile.NamedTemporaryFile("w", suffix=".json", encoding="utf-8") as handle:
            json.dump(schema, handle)
            handle.flush()
            subprocess.run(
                [
                    "datamodel-codegen",
                    "--input",
                    handle.name,
                    "--input-file-type",
                    "jsonschema",
                    "--output",
                    str(source),
                    "--output-model-type",
                    "pydantic_v2.BaseModel",
                    "--target-python-version",
                    "3.14",
                    "--use-standard-collections",
                    "--use-union-operator",
                    "--use-annotated",
                    "--disable-timestamp",
                ],
                check=True,
            )
        generated = source.read_text(encoding="utf-8").splitlines()
        if len(generated) > 1 and generated[1].startswith("#   filename:"):
            generated[1] = f"#   schema root: {root}"
        source.write_text("# ruff: noqa: E501\n" + "\n".join(generated) + "\n", encoding="utf-8")
        generated_name = "QueryResponse" if root.startswith("QueryResponseOf") else root
        exports.append(f"from .{module} import {generated_name} as {root}  # noqa: F401")
    (destination / "models.py").write_text(
        "# ruff: noqa: F401, I001\n" + "\n".join(exports) + "\n", encoding="utf-8"
    )


def camel_to_snake(value: str) -> str:
    """Convert a schema root name into a stable module name."""
    chars: list[str] = []
    for index, char in enumerate(value):
        if char.isupper() and index and not value[index - 1].isupper():
            chars.append("_")
        chars.append(char.lower())
    return "".join(chars)


def generate_client(commands: list[tuple[str, ...]], destination: Path) -> None:
    """Generate one typed method per discovered public command."""
    names = {"_".join(command): command for command in commands}
    missing = sorted(set(names) - set(RESPONSE_ROOTS))
    if missing:
        raise SystemExit(
            "client has no method for command: "
            + ", ".join(name.replace("_", " ") for name in missing)
        )
    lines = [
        '"""Generated typed client methods. Do not edit."""',
        "from __future__ import annotations",
        "",
        "from .models import (",
        *[f"    {root}," for root in sorted(set(RESPONSE_ROOTS.values()))],
        ")",
        "",
        "class GeneratedClient:",
        '    """Methods generated from the binary command surface."""',
        "",
        "    def _invoke[T](self, command: list[str], model: object, **options: object) -> T:",
        "        raise NotImplementedError",
        "",
    ]
    for name, command in sorted(names.items()):
        root = RESPONSE_ROOTS[name]
        return_type = RETURN_TYPES.get(name, root)
        lines.extend(
            [
                f"    def {name}(self, **options: object) -> {return_type}:",
                f'        """Run ``onetaskgraph {" ".join(command)}``."""',
                f"        return self._invoke({list(command)!r}, {return_type}, **options)",
                "",
            ]
        )
    (destination / "client.py").write_text("\n".join(lines), encoding="utf-8")
    (destination / "__init__.py").write_text(
        "from .client import GeneratedClient as GeneratedClient\n"
        "from .models import *  # noqa: F403\n",
        encoding="utf-8",
    )


def format_generated(destination: Path) -> None:
    """Apply the package's locked formatter to deterministic generated output."""
    subprocess.run(["ruff", "format", str(destination)], check=True, capture_output=True)


def main() -> None:
    """Regenerate, or compare regeneration with the committed package."""
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    bundle = json.loads(binary("schema"))
    commands = leaves()
    with tempfile.TemporaryDirectory() as temporary:
        target = Path(temporary) if args.check else GENERATED
        generate_models(bundle, target)
        generate_client(commands, target)
        format_generated(target)
        if args.check:
            expected = {
                path.name: path.read_text(encoding="utf-8")
                for path in target.iterdir()
                if path.is_file()
            }
            actual = {
                path.name: path.read_text(encoding="utf-8")
                for path in GENERATED.iterdir()
                if path.is_file()
            }
            if expected != actual:
                changed = sorted(
                    set(expected) ^ set(actual)
                    | {
                        name
                        for name in expected.keys() & actual.keys()
                        if expected[name] != actual[name]
                    }
                )
                raise SystemExit("generated Python SDK is stale: " + ", ".join(changed))


if __name__ == "__main__":
    main()
