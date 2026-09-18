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
    # The task response with the task's comments beside it, for a source that has them.
    "task_show": "TaskDetail",
    "task_deps": "QueryResponseOfQualifiedEdge",
    "task_copy": "CopyReport",
    "task_comment_add": "Comment",
    "task_comment_list": "CommentList",
    "task_comment_edit": "Comment",
    "task_comment_delete": "DeletedComment",
    "task_status_set": "TaskStatusSet",
    "task_metadata_set": "MetadataSet",
    "project_list": "QueryResponseOfQualifiedProject",
    "project_show": "QueryResponseOfQualifiedProject",
    "project_deps": "QueryResponseOfQualifiedEdge",
    "project_copy": "CopyReport",
    "project_metadata_set": "MetadataSet",
    "document_list": "QueryResponseOfQualifiedDocument",
    "document_show": "QueryResponseOfQualifiedDocument",
    "document_copy": "CopyReport",
    "document_metadata_set": "MetadataSet",
    "label_list": "QueryResponseOfQualifiedLabel",
    "search": "QueryResponseOfSearchHit",
    "sources_list": "SourceListing",
    "sources_status_options": "StatusOptionsReport",
    "config_show": "EffectiveConfig",
}
# Roots no command returns directly, which the package generates and exports anyway.
#
# Each is reachable inside a response and is named here so a consumer has it under its own
# name rather than only as a nested definition. `Location` is the one a caller most needs
# named: a document read answers with one, and acting on it means switching on which of its
# two keys is present. `DocumentQuery` and `PageOfDocument` are the plugin-facing halves of
# the same contract, which the SDK owes a caller a model for whether or not a verb returns
# one directly. `FailureDocument` is what any verb writes to stdout when it exits 1 under
# `--json`, which no verb's response root describes. `Delivered` and `DeliveryOutcome` are
# what a write reports about each task it kept in step with its deliverers, inside both a
# `CopyReport` and a `TaskStatusSet`; `TaskRef` is the entry of a task's `delivers` and
# `delivered_by`.
CONTRACT_ROOTS = {
    "FailureDocument",
    "SourceFailure",
    "QueryPlan",
    "GlobalId",
    "StatusCategory",
    "SourceName",
    "Document",
    "DocumentQuery",
    "Location",
    "PageOfDocument",
    "Delivered",
    "DeliveryOutcome",
    "TaskRef",
}
RETURN_TYPES = {"sources_list": "list[SourceListing]"}
OPTION_TYPES = {
    "apply": "bool",
    "allow_partial": "bool",
    "author": "str",
    "body_file": "str",
    "dry_run": "bool",
    "default_sources": "list[str] | tuple[str, ...]",
    "direction": "choices",
    "explain": "bool",
    "in_": "choices",
    "kind": "choices",
    "label": "list[str] | tuple[str, ...]",
    "limit": "int",
    "match_by": "str",
    "member": "list[GlobalId | str] | tuple[GlobalId | str, ...]",
    "no_project": "bool",
    "no_tasks": "bool",
    "not_label": "list[str] | tuple[str, ...]",
    "page": "str",
    "page_size": "int",
    "project": "str",
    "recreate": "bool",
    "search": "str",
    "set": "list[str] | tuple[str, ...]",
    "source": "list[str] | tuple[str, ...]",
    "status": "choice_list",
    "to": "str",
}
OPTION_PLACEHOLDERS = {
    "apply": None,
    "allow_partial": None,
    "author": "NAME",
    "body_file": "PATH",
    "dry_run": None,
    "default_sources": "NAMES",
    "direction": "DIRECTION",
    "explain": None,
    "in_": "FIELDS",
    "kind": "KIND",
    "label": "L",
    "limit": "N",
    "match_by": "KEY",
    "member": "TASK-ID",
    "no_project": None,
    "no_tasks": None,
    "not_label": "L",
    "page": "TOKEN",
    "page_size": "N",
    "project": "P",
    "recreate": None,
    "search": "TEXT",
    "set": "PATH=VALUE",
    "source": "S",
    "status": "S",
    "to": "SOURCE",
}


class SchemaBundle(TypedDict):
    """The validated portion of the emitted bundle generation consumes."""

    roots: dict[str, JsonValue]


# llmlint: ignore-block[async_typed_clients_at_boundaries] This is a build-time generator, not
# a client: one ordered pass that drives `cargo` as a subprocess and consumes its stdout as
# the schema bundle the next step reads. There is no concurrent work to overlap and no service
# on the other end, and nothing in this file ships in the wheel. The async typed boundary this
# rule asks of this package is `src/onetaskgraph_sdk/client.py`, which is exactly that.
def run_workspace_binary(*args: str) -> str:
    """Run the workspace binary, building the exact artifact under generation.

    A failure reports what the subprocess said. `check=True` raises with the command line
    alone, and the output this captures — stdout because it IS the answer, stderr with it —
    goes into that exception rather than to the terminal, so the whole diagnosis is lost.
    """
    command = ["cargo", "run", "--quiet", "-p", "onetaskgraph", "--bin", "onetaskgraph", "--"]
    command.extend(args)
    # The binary writes UTF-8 whatever the platform, and its schema descriptions carry
    # characters outside ASCII: decoding in the platform's code page, which `text=True` alone
    # does on the Windows runner, would hand every later step a mangled description.
    result = subprocess.run(
        command,
        cwd=ROOT.parent.parent,
        check=False,
        text=True,
        encoding="utf-8",
        capture_output=True,
    )
    if result.returncode != 0:
        rendered = " ".join(command)
        raise SystemExit(
            f"`{rendered}` exited {result.returncode}\n"
            f"--- its stderr ---\n{result.stderr}"
            f"--- its stdout ---\n{result.stdout}"
        )
    return result.stdout


# llmlint: ignore-end[async_typed_clients_at_boundaries]


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


def documented_minimum(schema: JsonValue, field: str) -> int:
    """Derive a numeric lower bound from the binary's schema description."""
    description = schema.get("description") if isinstance(schema, dict) else None
    match = re.search(r"At least (\d+)", description) if isinstance(description, str) else None
    if match is None:
        raise SystemExit(f"{field} schema did not document its minimum")
    return int(match.group(1))


def generate_models(bundle: SchemaBundle, destination: Path) -> None:
    """Generate Pydantic models directly from every response schema in the bundle."""
    destination.mkdir(parents=True, exist_ok=True)
    exports: list[str] = []
    for root in sorted(set(RESPONSE_ROOTS.values()) | CONTRACT_ROOTS):
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
            minimum = documented_minimum(page_size, "SourceListing.max_page_size")
            assert isinstance(page_size, dict)
            page_size["minimum"] = minimum
        module = camel_to_snake(root)
        source = destination / f"{module}.py"
        with tempfile.TemporaryDirectory() as temporary:
            schema_path = Path(temporary) / f"{module}.json"
            schema_path.write_text(json.dumps(schema), encoding="utf-8")
            subprocess.run(
                [
                    "datamodel-codegen",
                    "--input",
                    str(schema_path),
                    "--input-file-type",
                    "jsonschema",
                    "--output",
                    str(source),
                    "--output-model-type",
                    "pydantic_v2.BaseModel",
                    "--target-python-version",
                    "3.13",
                    "--use-standard-collections",
                    "--use-union-operator",
                    "--use-annotated",
                    "--use-title-as-name",
                    "--disable-timestamp",
                    # Named rather than left to the tool's default, because this module is
                    # read back below as UTF-8 and compared byte for byte by `--check`.
                    "--encoding",
                    "utf-8",
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
        if root == "MetadataSet":
            generated = any_json_value(
                generated,
                "    # A metadata value is arbitrary JSON by the emitted wire contract: the key's\n"
                "    # value as the source reads it back, of whatever JSON type the caller set.",
            )
        if any("dict[str, Any]" in line for line in generated):
            generated = [
                line.replace("from pydantic import ", "from pydantic import JsonValue, ").replace(
                    "dict[str, Any]", "dict[str, JsonValue]"
                )
                for line in generated
            ]
        source.write_text(
            "# ruff: noqa: E501  # Generated descriptions preserve the schema's text.\n"
            + "\n".join(generated)
            + "\n",
            encoding="utf-8",
        )
        generated_name = root
        for generic, titled in (("QueryResponseOf", "QueryResponse"), ("PageOf", "Page")):
            if root.startswith(generic):
                generated_name = titled
        exports.append(f"from .{module} import {generated_name} as {root}")
    (destination / "models.py").write_text(
        "# ruff: noqa: F401, I001  # Generated public re-exports are used by consumers.\n"
        + "\n".join(exports)
        + "\n",
        encoding="utf-8",
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
                # The wire contract deliberately uses `created` for a real create and a
                # dry run that would create. Keep the generated model's name truthful
                # without changing the serialized action Rust and CLI consumers read.
                if hint == "CopyOutcome" and discriminant == "created":
                    words = "CreatedOrWouldCreate"
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


def any_json_value(lines: list[str], reason: str) -> list[str]:
    """Put `reason` above a `value` field the code generator typed `Any`, saying why it is.

    A schema that constrains nothing accepts any JSON value, and `Any` is the only annotation
    that says so; the comment is what tells a reader the escape is the contract rather than a
    gap in it.
    """
    annotated: list[str] = []
    for index, line in enumerate(lines):
        following = lines[index + 1].strip() if index + 1 < len(lines) else ""
        if line.startswith("    value: Annotated[Any") or (
            line == "    value: Annotated[" and following == "Any,"
        ):
            annotated.append(reason)
        annotated.append(line)
    return annotated


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


def operands(command: tuple[str, ...]) -> tuple[str, ...]:
    """Name the positionals one command takes ahead of its options, in the order it takes them."""
    match command:
        case ("search",):
            return ("text",)
        case ("sources", "status-options"):
            return ("source",)
        case ("task" | "document", "copy"):
            return ("ids",)
        case ("task", "comment", "add" | "list"):
            return ("id",)
        case ("task", "comment", "edit" | "delete"):
            return ("id", "comment_id")
        case ("task", "status", "set"):
            return ("id", "category")
        case ("task" | "project" | "document", "metadata", "set"):
            return ("id", "key", "value")
        case ("task" | "project" | "document", "show" | "deps" | "copy"):
            return ("id",)
        case _:
            return ()


# The commands that read a body from standard input when `--body-file` is absent, which
# generated methods expose as a `body` the client writes there — a body is never a word of
# the command line, so a caller holding text needs no file to pass it.
BODY_COMMANDS = {("task", "comment", "add"), ("task", "comment", "edit")}


def generate_client(commands: list[tuple[str, ...]], destination: Path) -> None:
    """Generate one typed method per discovered public command."""
    names = {"_".join(command).replace("-", "_"): command for command in commands}
    missing = sorted(set(names) - set(RESPONSE_ROOTS))
    if missing:
        raise SystemExit(
            "client has no method for command: "
            + ", ".join(name.replace("_", " ") for name in missing)
        )
    positionals = {command: operands(command) for command in commands if operands(command)}
    lines = [
        '"""Generated typed client methods. Do not edit."""',
        "from __future__ import annotations",
        "",
        "from typing import Literal",
        "",
        "from .models import (",
        *[
            f"    {root},"
            for root in sorted(
                set(RESPONSE_ROOTS.values()) | {"GlobalId", "SourceName", "StatusCategory"}
            )
        ],
        ")",
        "",
        "POSITIONALS: dict[tuple[str, ...], tuple[str, ...]] = {",
        *[
            f"    {command!r}: {positional!r},"
            for command, positional in sorted(positionals.items())
        ],
        "}",
        '"""The operands each command takes ahead of its options, in order, by command.',
        "",
        "The runtime client builds the argument vector from this rather than from a second",
        "table of its own: a verb whose operand was named in one place and forgotten in the",
        "other generates a method that cannot do what it is named for, and nothing would say",
        "so until the binary refused the invocation.",
        '"""',
        "",
        "class GeneratedClient:",
        '    """Methods generated from the binary command surface."""',
        "",
        "    async def _invoke[T](",
        "        self,",
        "        command: list[str],",
        "        model: object,",
        "        *,",
        "        stdin: str | None = None,",
        "        **options: object,",
        "    ) -> T:",
        "        raise NotImplementedError",
        "",
    ]
    positional_types = {
        "id": "GlobalId | str",
        # `task copy` and `document copy` take one or more ids, which are the variadic
        # positionals the command surface has; the client passes each of them through.
        "ids": "list[GlobalId | str] | tuple[GlobalId | str, ...]",
        # `task status set` takes the category it sets as its second operand, spelled as the
        # binary's status vocabulary spells it — which is exactly the generated enum's values.
        "category": "StatusCategory | str",
        "source": "SourceName | str",
        # `metadata set` takes its key as a string, and its value as the JSON text the binary
        # parses strictly — exactly the word the command line takes, so what a caller writes
        # is what the binary reads, and a value is never re-encoded on its way there.
        "key": "str",
        "value": "str",
    }
    for name, command in sorted(names.items()):
        root = RESPONSE_ROOTS[name]
        return_type = RETURN_TYPES.get(name, root)
        taken = positionals.get(command, ())
        keywords = [item for item in option_names(command) if item not in taken]
        body = ["body: str | None = None"] if command in BODY_COMMANDS else []
        parameters = (
            [f"{positional}: {positional_types.get(positional, 'str')}" for positional in taken]
            + ["*"]
            + [f"{item}: {option_type(command, item)} | None = None" for item in keywords]
            + body
        )
        if parameters[-1] == "*":
            parameters.pop()
        passed = [f"{item}={item}" for item in [*taken, *keywords]]
        if body:
            passed.append("stdin=body")
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


def enum_defaults(lines: list[str]) -> list[str]:
    """Render a defaulted enum field as its member rather than as the raw schema value.

    The code generator writes a schema `default` back as the JSON literal it was, so a
    field annotated with a generated `StrEnum` is assigned a bare string — which the type
    checker rejects, because a `str` is not that enum. The bundle carries one such field
    today (`Capabilities.documents`, `"unsupported"` when a plugin omits it) and will carry
    more the day another optional enum member lands, so this rewrites by rule rather than
    by name.

    A default this cannot place is left exactly as it was. That is deliberate: the type
    check is what caught this one, and it is what should catch the next one rather than a
    silent rewrite here that guesses wrong.
    """
    members: dict[str, dict[str, str]] = {}
    enum_name: str | None = None
    for line in lines:
        if declared := re.fullmatch(r"class (\w+)\(StrEnum\):", line):
            enum_name = declared.group(1)
            members[enum_name] = {}
        elif enum_name is not None:
            if member := re.fullmatch(r"    (\w+) = \"(.*)\"", line):
                members[enum_name][member.group(2)] = member.group(1)
            elif line.strip():
                enum_name = None

    rewritten: list[str] = []
    annotated: str | None = None
    for line in lines:
        if re.fullmatch(r"    \w+: Annotated\[", line):
            annotated = ""
        elif annotated == "" and (typed := re.fullmatch(r"        (\w+)(?: \| None)?,", line)):
            annotated = typed.group(1)
        elif assigned := re.fullmatch(r"    \] = \"(.*)\"", line):
            member = members.get(annotated or "", {}).get(assigned.group(1))
            if member is not None:
                line = f"    ] = {annotated}.{member}"
            annotated = None
        rewritten.append(line)
    return rewritten


def format_generated(destination: Path) -> None:
    """Apply the package's locked formatter to deterministic generated output."""
    subprocess.run(["ruff", "format", str(destination)], check=True, capture_output=True)
    subprocess.run(
        # `I001` alongside `F401` because the package's own lint enforces import order and
        # the code generator does not: it emits imports in the order it happens to need
        # them, so a root that gains a type can reorder them into something `ruff check`
        # then refuses. Sorting here keeps generated output passing the same lint every
        # hand-written module passes.
        ["ruff", "check", "--fix", "--select", "F401,I001", str(destination)],
        check=True,
    )
    # After formatting rather than before it: the shape a field is written in is the
    # formatter's, and reading it back is what lets this be one rule rather than a guess
    # at what the code generator happened to emit on one line or several.
    for module in sorted(destination.glob("*.py")):
        lines = module.read_text(encoding="utf-8").splitlines()
        rewritten = enum_defaults(lines)
        if rewritten != lines:
            module.write_text("\n".join(rewritten) + "\n", encoding="utf-8")
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
        raise SystemExit(
            "generated Python SDK is stale: "
            + ", ".join(changed)
            + "; run `uv run python generate.py` from sdks/python to regenerate"
        )


def validate_schema_bundle(parsed: JsonValue) -> SchemaBundle:
    """Validate the binary's schema output before generation consumes a root."""
    if not isinstance(parsed, dict) or not isinstance(parsed.get("roots"), dict):
        raise SystemExit("binary emitted an invalid schema bundle: expected an object with roots")
    bundle = TypeAdapter(SchemaBundle).validate_python(parsed)
    required = set(RESPONSE_ROOTS.values()) | CONTRACT_ROOTS
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


def generate(bundle: SchemaBundle, *, check: bool, destination: Path = GENERATED) -> None:
    """Write or check generated output for one validated bundle."""
    commands = leaves()
    with tempfile.TemporaryDirectory() as temporary:
        target = Path(temporary) if check else destination
        generate_models(bundle, target)
        generate_client(commands, target)
        format_generated(target)
        if check:
            check_generated(target, destination)


def main() -> None:
    """Regenerate, or compare regeneration with the committed package."""
    parser = argparse.ArgumentParser()
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    parsed = json.loads(run_workspace_binary("schema"))
    generate(validate_schema_bundle(parsed), check=args.check)


if __name__ == "__main__":
    main()
