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
OPTION_TYPES = {
    "allow_partial": "bool",
    "default_sources": "list[str] | tuple[str, ...]",
    "direction": "choices",
    "explain": "bool",
    "in_": "choices",
    "kind": "choices",
    "label": "list[str] | tuple[str, ...]",
    "limit": "int",
    "no_project": "bool",
    "not_label": "list[str] | tuple[str, ...]",
    "page": "str",
    "page_size": "int",
    "project": "str",
    "search": "str",
    "set": "list[str] | tuple[str, ...]",
    "source": "list[str] | tuple[str, ...]",
    "status": "choice_list",
}
OPTION_PLACEHOLDERS = {
    "allow_partial": None,
    "default_sources": "NAMES",
    "direction": "DIRECTION",
    "explain": None,
    "in_": "FIELDS",
    "kind": "KIND",
    "label": "L",
    "limit": "N",
    "no_project": None,
    "not_label": "L",
    "page": "TOKEN",
    "page_size": "N",
    "project": "P",
    "search": "TEXT",
    "set": "PATH=VALUE",
    "source": "S",
    "status": "S",
}


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
    discovered = re.findall(
        r"^\s+--([a-z][a-z-]*)(?: <([^>]+)>)?",
        run_workspace_binary(*command, "--help"),
        re.MULTILINE,
    )
    names = [name for name, _ in discovered]
    normalized = {name.replace("-", "_") for name in names} - {"help", "json", "output"}
    result = sorted(f"{name}_" if keyword.iskeyword(name) else name for name in normalized)
    placeholders = {
        (
            f"{name.replace('-', '_')}_"
            if keyword.iskeyword(name.replace("-", "_"))
            else name.replace("-", "_")
        ): (placeholder or None)
        for name, placeholder in discovered
        if name not in {"help", "json", "output"}
    }
    validate_option_placeholders(placeholders, result)
    return result


def validate_option_placeholders(placeholders: dict[str, str | None], names: list[str]) -> None:
    """Reject help whose option value shapes drifted from generated typing."""
    for name in names:
        if placeholders[name] != OPTION_PLACEHOLDERS[name]:
            raise SystemExit(
                f"binary changed the value shape for option --{name.replace('_', '-')}"
            )


def option_type(command: tuple[str, ...], name: str) -> str:
    """Derive finite option domains from clap help and scalar shapes from placeholders."""
    configured = OPTION_TYPES[name]
    if configured not in {"choices", "choice_list"}:
        return configured
    cli_name = name.removesuffix("_").replace("_", "-")
    help_text = run_workspace_binary(*command, "--help")
    choices = choice_values(help_text, cli_name)
    literal = "Literal[" + ", ".join(repr(choice) for choice in choices) + "]"
    return f"list[{literal}] | tuple[{literal}, ...]" if configured == "choice_list" else literal


def choice_values(help_text: str, cli_name: str) -> list[str]:
    """Read one finite option vocabulary from clap's emitted help."""
    _, separator, block = help_text.partition(f"--{cli_name} ")
    if not separator:
        raise SystemExit(f"binary did not report option --{cli_name} in command help")
    block = re.split(r"\n\s+(?:--|-h, --)", block, maxsplit=1)[0]
    choices = re.findall(r"^\s+- ([a-z0-9-]+):", block, re.MULTILINE)
    if not choices:
        raise SystemExit(f"binary did not report possible values for option --{cli_name}")
    return choices


def generate_models(bundle: SchemaBundle, destination: Path) -> None:
    """Generate Pydantic models directly from every response schema in the bundle."""
    destination.mkdir(parents=True, exist_ok=True)
    exports: list[str] = []
    for root in sorted(
        set(RESPONSE_ROOTS.values()) | {"SourceFailure", "QueryPlan", "GlobalId", "StatusCategory"}
    ):
        schema = bundle["roots"][root]
        add_variant_titles(schema, root)
        rename_qualified_definitions(schema)
        if root == "SourceListing":
            definitions = schema.get("$defs") if isinstance(schema, dict) else None
            capabilities = (
                definitions.get("Capabilities") if isinstance(definitions, dict) else None
            )
            properties = capabilities.get("properties") if isinstance(capabilities, dict) else None
            page_size = properties.get("max_page_size") if isinstance(properties, dict) else None
            if isinstance(page_size, dict):
                # Implementations reject zero; preserve that runtime invariant at this boundary.
                page_size["minimum"] = 1
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
    match value:
        case list(items):
            for item in items:
                add_variant_titles(item, hint)
            return
        case dict(mapping):
            value = mapping
        case _:
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
    match value:
        case list(items):
            for item in items:
                replace_references(item, renames)
        case dict(mapping):
            reference = mapping.get("$ref")
            if isinstance(reference, str):
                tail = reference.rsplit("/", 1)[-1]
                if tail in renames:
                    mapping["$ref"] = reference.rsplit("/", 1)[0] + "/" + renames[tail]
            for child in mapping.values():
                replace_references(child, renames)
        case _:
            return


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
        "from typing import Literal",
        "",
        "from .models import (",
        *[
            f"    {root},"
            for root in sorted(set(RESPONSE_ROOTS.values()) | {"GlobalId", "StatusCategory"})
        ],
        ")",
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
        match command:
            case ("search",):
                positional = "text"
            case ("task" | "project", "show" | "deps"):
                positional = "id"
            case _:
                positional = None
        keywords = [item for item in option_names(command) if item != positional]
        positional_type = "GlobalId | str" if positional == "id" else "str"
        parameters = (
            ([f"{positional}: {positional_type}"] if positional else [])
            + ["*"]
            + [f"{item}: {option_type(command, item)} | None = None" for item in keywords]
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
    subprocess.run(
        ["ruff", "check", "--fix", "--select", "F401", str(destination)],
        check=True,
    )
    subprocess.run(["ruff", "format", str(destination)], check=True, capture_output=True)


def check_generated(expected_dir: Path, actual_dir: Path) -> None:
    """Reject a generated directory that differs from the expected output."""
    expected = {
        path.name: path.read_text(encoding="utf-8")
        for path in expected_dir.iterdir()
        if path.is_file()
    }
    actual = {
        path.name: path.read_text(encoding="utf-8")
        for path in actual_dir.iterdir()
        if path.is_file()
    }
    changed = sorted(
        set(expected) ^ set(actual)
        | {name for name in expected.keys() & actual.keys() if expected[name] != actual[name]}
    )
    if changed:
        raise SystemExit("generated Python SDK is stale: " + ", ".join(changed))


def validate_schema_bundle(parsed: JsonValue) -> SchemaBundle:
    """Validate the binary's schema output before generation consumes a root."""
    if not isinstance(parsed, dict) or not isinstance(parsed.get("roots"), dict):
        raise SystemExit("binary emitted an invalid schema bundle: expected an object with roots")
    bundle = TypeAdapter(SchemaBundle).validate_python(parsed)
    required = set(RESPONSE_ROOTS.values()) | {
        "SourceFailure",
        "QueryPlan",
        "GlobalId",
        "StatusCategory",
    }
    missing = sorted(required - bundle["roots"].keys())
    malformed = sorted(
        name
        for name in required & bundle["roots"].keys()
        if not isinstance(bundle["roots"][name], dict)
    )
    if missing or malformed:
        details = [f"missing roots: {', '.join(missing)}"] if missing else []
        details += [f"non-object roots: {', '.join(malformed)}"] if malformed else []
        raise SystemExit("binary emitted an invalid schema bundle: " + "; ".join(details))
    return bundle


def generate(bundle: SchemaBundle, *, check: bool) -> None:
    """Write or check generated output for one validated bundle."""
    commands = leaves()
    with tempfile.TemporaryDirectory() as temporary:
        target = Path(temporary) if check else GENERATED
        generate_models(bundle, target)
        generate_client(commands, target)
        format_generated(target)
        if check:
            check_generated(target, GENERATED)


def main() -> None:
    """Regenerate, or compare regeneration with the committed package."""
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    parsed = json.loads(run_workspace_binary("schema"))
    generate(validate_schema_bundle(parsed), check=args.check)


if __name__ == "__main__":
    main()
