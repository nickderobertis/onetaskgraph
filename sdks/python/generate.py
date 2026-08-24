"""Generate the Python contract and client surface from the running binary."""

from __future__ import annotations

import argparse
import json
import keyword
import re
import subprocess
import tempfile
from pathlib import Path
from typing import TypedDict

from pydantic import JsonValue, TypeAdapter

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


class SchemaBundle(TypedDict):
    """The validated portion of the emitted bundle generation consumes."""

    roots: dict[str, JsonValue]


def run_workspace_binary(*args: str) -> str:
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
    help_text = run_workspace_binary(*prefix, "--help")
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
        child_help = run_workspace_binary(*child, "--help")
        if "Commands:\n" in child_help:
            found.extend(leaves(child))
        else:
            found.append(child)
    return found


def option_names(command: tuple[str, ...]) -> list[str]:
    """Derive keyword names from clap's help for one discovered leaf command."""
    names = re.findall(
        r"^\s+--([a-z][a-z-]*)", run_workspace_binary(*command, "--help"), re.MULTILINE
    )
    normalized = {name.replace("-", "_") for name in names} - {"help", "json", "output"}
    return sorted(f"{name}_" if keyword.iskeyword(name) else name for name in normalized)


def generate_models(bundle: SchemaBundle, destination: Path) -> None:
    """Generate Pydantic models directly from every response schema in the bundle."""
    destination.mkdir(parents=True, exist_ok=True)
    exports: list[str] = []
    for root in sorted(set(RESPONSE_ROOTS.values()) | {"SourceFailure", "QueryPlan"}):
        schema = bundle["roots"][root]
        add_variant_titles(schema, root)
        rename_qualified_definitions(schema)
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
                    "--use-title-as-name",
                    "--disable-timestamp",
                ],
                check=True,
            )
        generated = source.read_text(encoding="utf-8").splitlines()
        if len(generated) > 1 and generated[1].startswith("#   filename:"):
            generated[1] = f"#   schema root: {root}"
        if root == "EffectiveConfig":
            generated = [
                line.replace(
                    "    value: Annotated[Any",
                    "    # A setting is arbitrary JSON by the emitted wire contract.\n"
                    "    value: Annotated[Any",
                )
                for line in generated
            ]
        source.write_text(
            "# ruff: noqa: E501  # Generated descriptions preserve the schema's text.\n"
            + "\n".join(generated)
            + "\n",
            encoding="utf-8",
        )
        generated_name = "QueryResponse" if root.startswith("QueryResponseOf") else root
        exports.append(f"from .{module} import {generated_name} as {root}")
    (destination / "models.py").write_text(
        "# ruff: noqa: F401, I001  # Generated public re-exports are used by consumers.\n"
        + "\n".join(exports)
        + "\n",
    )


def add_variant_titles(value: JsonValue, hint: str) -> None:
    """Give anonymous schema variants stable domain names before model generation."""
    if isinstance(value, list):
        for item in value:
            add_variant_titles(item, hint)
        return
    if not isinstance(value, dict):
        return
    variants = value.get("oneOf")
    if isinstance(variants, list):
        for variant in variants:
            if not isinstance(variant, dict) or "title" in variant:
                continue
            discriminant = variant.get("const")
            properties = variant.get("properties")
            if discriminant is None and isinstance(properties, dict):
                for property_schema in properties.values():
                    if isinstance(property_schema, dict) and "const" in property_schema:
                        discriminant = property_schema["const"]
                        break
            if isinstance(discriminant, str):
                words = "".join(part.title() for part in discriminant.split("-"))
                variant["title"] = f"{hint}{words}"
    for key, child in value.items():
        bare = key.removeprefix("$")
        child_hint = bare if any(char.isupper() for char in bare) else bare.title()
        add_variant_titles(child, child_hint or hint)


def rename_qualified_definitions(value: JsonValue) -> None:
    """Name generic qualified definitions after the task or project they contain."""
    if not isinstance(value, dict):
        return
    definitions = value.get("$defs")
    if not isinstance(definitions, dict):
        return
    renames: dict[str, str] = {}
    for name, definition in definitions.items():
        if not name.startswith("Qualified") or not isinstance(definition, dict):
            continue
        properties = definition.get("properties")
        item = properties.get("item") if isinstance(properties, dict) else None
        reference = item.get("$ref") if isinstance(item, dict) else None
        if isinstance(reference, str):
            renames[name] = f"Qualified{reference.rsplit('/', 1)[-1]}"
    for old, new in renames.items():
        definitions[new] = definitions.pop(old)
    replace_references(value, renames)


def replace_references(value: JsonValue, renames: dict[str, str]) -> None:
    """Update local references after a generated-definition rename."""
    if isinstance(value, list):
        for item in value:
            replace_references(item, renames)
    elif isinstance(value, dict):
        reference = value.get("$ref")
        if isinstance(reference, str):
            tail = reference.rsplit("/", 1)[-1]
            if tail in renames:
                value["$ref"] = reference.rsplit("/", 1)[0] + "/" + renames[tail]
        for child in value.values():
            replace_references(child, renames)


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
        "type Option = str | int | bool | list[str] | tuple[str, ...] | None",
        "",
        "class GeneratedClient:",
        '    """Methods generated from the binary command surface."""',
        "",
        "    async def _invoke[T](",
        "        self, command: list[str], model: object, **options: object",
        "    ) -> T:",
        "        raise NotImplementedError",
        "",
    ]
    for name, command in sorted(names.items()):
        root = RESPONSE_ROOTS[name]
        return_type = RETURN_TYPES.get(name, root)
        positional = (
            "text"
            if command == ("search",)
            else "id"
            if command[0] in {"task", "project"} and command[-1] in {"show", "deps"}
            else None
        )
        keywords = [item for item in option_names(command) if item != positional]
        parameters = (
            ([f"{positional}: str"] if positional else [])
            + ["*"]
            + [f"{item}: Option = None" for item in keywords]
        )
        if parameters[-1] == "*":
            parameters.pop()
        passed = [f"{item}={item}" for item in ([positional] if positional else []) + keywords]
        lines.extend(
            [
                f"    async def {name}(self, {', '.join(parameters)}) -> {return_type}:",
                f'        """Run ``onetaskgraph {" ".join(command)}``."""',
                "        return await self._invoke("
                f"{list(command)!r}, {return_type}, {', '.join(passed)})",
                "",
            ]
        )
    (destination / "client.py").write_text("\n".join(lines), encoding="utf-8")
    (destination / "__init__.py").write_text(
        "from .client import GeneratedClient as GeneratedClient\n"
        "from .models import *  # noqa: F403  # Schema roots define the public set.\n",
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
    parsed = json.loads(run_workspace_binary("schema"))
    if not isinstance(parsed, dict) or not isinstance(parsed.get("roots"), dict):
        raise SystemExit("binary emitted an invalid schema bundle: expected an object with roots")
    bundle = TypeAdapter(SchemaBundle).validate_python(parsed)
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
