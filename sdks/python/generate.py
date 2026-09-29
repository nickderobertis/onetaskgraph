"""Generate the Python contract and client surface from the running binary."""

from __future__ import annotations

import argparse
import importlib.util
import inspect
import json
import keyword
import re
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import NamedTuple, TypedDict

from pydantic import BaseModel, JsonValue, RootModel, TypeAdapter, ValidationError

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
    "task_priority_set": "TaskPrioritySet",
    "task_content_set": "TaskContentSet",
    "task_metadata_set": "MetadataSet",
    "task_update": "TaskUpdated",
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
    "sources_fields": "FieldsReport",
    "config_show": "EffectiveConfig",
    "template_variables": "TemplateVariables",
    "template_render": "RenderedTemplate",
    # A created task as `task show` answers with it, and a created document as `document show`.
    "task_create": "TaskDetail",
    "task_render": "Regenerated",
    "task_answers": "TemplateAnswers",
    "document_create": "QueryResponseOfQualifiedDocument",
    "document_render": "Regenerated",
    "document_answers": "TemplateAnswers",
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
# `delivered_by`. `TemplateVariable`, `VariableType` and `ItemType` are one entry of what
# `template variables` answers and the two vocabularies it types a variable with.
# `TemplateProvenance` is the `onetaskgraph.template` entry a rendered item's metadata holds,
# which a caller checking for a hand edit or a changed template reads by name. `UpdatedField` is
# the vocabulary `task update` reports what it wrote in, which a caller branches on by name.
CONTRACT_ROOTS = {
    "FailureDocument",
    "SourceFailure",
    "QueryPlan",
    "GlobalId",
    "StatusCategory",
    "Priority",
    "SourceName",
    "Document",
    "DocumentQuery",
    "Location",
    "PageOfDocument",
    "Delivered",
    "DeliveryOutcome",
    "TaskRef",
    "TemplateVariable",
    "VariableType",
    "ItemType",
    "TemplateProvenance",
    "UpdatedField",
}
RETURN_TYPES = {"sources_list": "list[SourceListing]"}
OPTION_TYPES = {
    "apply": "bool",
    "allow_partial": "bool",
    "answers": "answers",
    "author": "str",
    "body_file": "str",
    "dry_run": "bool",
    "default_sources": "list[str] | tuple[str, ...]",
    "delivers": "list[GlobalId | str] | tuple[GlobalId | str, ...]",
    "depends_on": "list[GlobalId | str] | tuple[GlobalId | str, ...]",
    "direction": "choices",
    "explain": "bool",
    "file": "str",
    "id": "str",
    "in_": "choices",
    "kind": "choices",
    "label": "list[str] | tuple[str, ...]",
    "limit": "int",
    "match_by": "str",
    "member": "list[GlobalId | str] | tuple[GlobalId | str, ...]",
    "metadata": "list[str] | tuple[str, ...]",
    "no_delivers": "bool",
    "no_depends_on": "bool",
    "no_project": "bool",
    "no_tasks": "bool",
    "not_label": "list[str] | tuple[str, ...]",
    "page": "str",
    "page_size": "int",
    "priority": "choice_list",
    "project": "str",
    "recreate": "bool",
    "remove_metadata": "list[str] | tuple[str, ...]",
    "repository": "list[str] | tuple[str, ...]",
    "search": "str",
    "search_path": "list[str] | tuple[str, ...]",
    "set": "list[str] | tuple[str, ...]",
    "source": "list[str] | tuple[str, ...]",
    "status": "choice_list",
    "status_name": "str",
    "template": "str",
    "template_loader": "str",
    "title": "str",
    "to": "str",
    "unset": "list[str] | tuple[str, ...]",
    "var": "list[str] | tuple[str, ...]",
}
OPTION_PLACEHOLDERS = {
    "apply": None,
    "allow_partial": None,
    "answers": "FILE",
    "author": "NAME",
    "body_file": "PATH",
    "dry_run": None,
    "default_sources": "NAMES",
    "delivers": "ID",
    "depends_on": "ID",
    "direction": "DIRECTION",
    "explain": None,
    "file": "PATH",
    "id": "DOC",
    "in_": "FIELDS",
    "kind": "KIND",
    "label": "L",
    "limit": "N",
    "match_by": "KEY",
    "member": "TASK-ID",
    "metadata": "KEY=JSON",
    "no_delivers": None,
    "no_depends_on": None,
    "no_project": None,
    "no_tasks": None,
    "not_label": "L",
    "page": "TOKEN",
    "page_size": "N",
    "priority": "PRIORITY",
    "project": "P",
    "recreate": None,
    "remove_metadata": "KEY",
    "repository": "R",
    "search": "TEXT",
    "search_path": "DIR",
    "set": "PATH=VALUE",
    "source": "S",
    "status": "S",
    "status_name": "NAME",
    "template": "FILE",
    "template_loader": "FILE",
    "title": "TITLE",
    "to": "SOURCE",
    "unset": "NAME",
    "var": "NAME=VALUE",
}


class OptionShape(NamedTuple):
    """How one command's help spells an option: the type generated for it, and its placeholder."""

    type: str
    placeholder: str | None


# Options one command spells differently from every other that shares their name, by command.
# `task create` takes one `--status`, the category the task is created in, where every list verb
# takes several to filter by.
COMMAND_OPTIONS: dict[tuple[str, ...], dict[str, OptionShape]] = {
    ("task", "create"): {"status": OptionShape(type="choices", placeholder="CATEGORY")},
    # `task update` names one status and one priority, the ones it writes.
    ("task", "update"): {
        "status": OptionShape(type="choices", placeholder="CATEGORY"),
        "priority": OptionShape(type="choices", placeholder="PRIORITY"),
    },
}


# Global flags no generated method takes: `--json` and `--output`, because the client always
# asks for machine output, and `--interactive` / `--no-interactive`, because the client always
# passes `--no-interactive` — a library call must never wait on a prompt nobody can answer.
UNEXPOSED_OPTIONS = {"help", "json", "output", "interactive", "no_interactive"}


class SchemaBundle(TypedDict):
    """The validated portion of the emitted bundle generation consumes."""

    roots: dict[str, JsonValue]


BINARY = (
    ROOT.parent.parent
    / "target"
    / "debug"
    / ("onetaskgraph.exe" if sys.platform == "win32" else "onetaskgraph")
)
"""The executable `onetaskgraph:build` produces, which is the artifact under generation.

Nothing here builds it: scripts/check-workspace-config.sh says why a spawner never does.
"""


# llmlint: ignore-block[async_typed_clients_at_boundaries] This is a build-time generator, not
# a client: one ordered pass that drives the built binary as a subprocess and consumes its
# stdout as the schema bundle the next step reads. There is no concurrent work to overlap and
# no service on the other end, and nothing in this file ships in the wheel. The async typed
# boundary this rule asks of this package is `src/onetaskgraph_sdk/client.py`, which is
# exactly that.
def run_workspace_binary(*args: str) -> str:
    """Run the workspace binary that `onetaskgraph:build` produced.

    A failure reports what the subprocess said. `check=True` raises with the command line
    alone, and the output this captures — stdout because it IS the answer, stderr with it —
    goes into that exception rather than to the terminal, so the whole diagnosis is lost.
    """
    if not BINARY.is_file():
        raise SystemExit(
            f"{BINARY} is missing; run `scripts/nx.sh run onetaskgraph:build` from the "
            "workspace root, which is what every Nx target that spawns it depends on"
        )
    command = [str(BINARY), *args]
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
    normalized = {name.replace("-", "_") for name in names} - UNEXPOSED_OPTIONS
    result = sorted(f"{name}_" if keyword.iskeyword(name) else name for name in normalized)
    placeholders = {
        (
            f"{name.replace('-', '_')}_"
            if keyword.iskeyword(name.replace("-", "_"))
            else name.replace("-", "_")
        ): (placeholder or None)
        for name, placeholder in discovered
        if name.replace("-", "_") not in UNEXPOSED_OPTIONS
    }
    validate_option_placeholders(placeholders, result, command)
    return result


def validate_option_placeholders(
    placeholders: dict[str, str | None], names: list[str], command: tuple[str, ...] = ()
) -> None:
    """Reject help whose option value shapes drifted from generated typing."""
    overrides = COMMAND_OPTIONS.get(command, {})
    for name in names:
        expected = overrides[name].placeholder if name in overrides else OPTION_PLACEHOLDERS[name]
        if placeholders[name] != expected:
            raise SystemExit(
                f"binary changed the value shape for option --{name.replace('_', '-')}"
            )


def option_type(command: tuple[str, ...], name: str) -> str:
    """Derive finite option domains from clap help and scalar shapes from placeholders."""
    overrides = COMMAND_OPTIONS.get(command, {})
    configured = overrides[name].type if name in overrides else OPTION_TYPES[name]
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
    roots = sorted(set(RESPONSE_ROOTS.values()) | CONTRACT_ROOTS)
    for root in roots:
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
                    # A member the schema gives a default and no `null` type is generated as
                    # its own type with that default, rather than `T | None`: the wire lets it
                    # be omitted, never written as `null`. A member whose schema admits `null`
                    # (an `Option` on the Rust side) stays optional. [`nullability_disagreements`]
                    # holds the result to the schema.
                    "--strict-nullable",
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
        if root in {"TemplateVariable", "TemplateVariables"}:
            generated = any_json_value(
                generated,
                "    # A default is arbitrary JSON by the emitted wire contract: a value of the\n"
                "    # variable's own `type`, which the declaration beside it names.",
                field="default",
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
        + "\n\n"
        + "# Every root is named here rather than left to the `import X as X` form alone: a\n"
        + "# root whose generated class carries another name — every `QueryResponseOf…`, and\n"
        + "# `PageOfDocument` — is aliased, and a strict type checker reads an aliased name as\n"
        + "# private to this module rather than as a re-export. This list is what makes the\n"
        + "# whole set public to one, here and through the package's own `import *`.\n"
        + "__all__ = [\n"
        + "".join(f'    "{root}",\n' for root in roots)
        + "]\n",
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


def any_json_value(lines: list[str], reason: str, field: str = "value") -> list[str]:
    """Put `reason` above a `field` the code generator typed `Any`, saying why it is.

    A schema that constrains nothing accepts any JSON value, and `Any` is the only annotation
    that says so; the comment is what tells a reader the escape is the contract rather than a
    gap in it.
    """
    annotated: list[str] = []
    for index, line in enumerate(lines):
        following = lines[index + 1].strip() if index + 1 < len(lines) else ""
        if line.startswith(f"    {field}: Annotated[Any") or (
            line == f"    {field}: Annotated[" and following.startswith("Any")
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
        case ("sources", "status-options" | "fields"):
            return ("source",)
        case ("task" | "document", "copy"):
            return ("ids",)
        case ("task", "comment", "add" | "list"):
            return ("id",)
        case ("task", "comment", "edit" | "delete"):
            return ("id", "comment_id")
        case ("task", "status", "set"):
            return ("id", "category")
        case ("task", "priority", "set"):
            return ("id", "priority")
        case ("task", "content", "set"):
            return ("id",)
        case ("task", "update"):
            return ("id",)
        case ("task" | "project" | "document", "metadata", "set"):
            return ("id", "key", "value")
        case ("task" | "project" | "document", "show" | "deps" | "copy"):
            return ("id",)
        case ("template", "variables" | "render"):
            return ("file",)
        case ("task" | "document", "create"):
            return ("source",)
        case ("task" | "document", "render" | "answers"):
            return ("id",)
        case _:
            return ()


# Options a command cannot run without, which its generated method takes as required
# parameters beside its operands rather than as optional keywords: `task content set` has
# nothing to write without `--file`, and a create nothing to file without `--project` and
# `--title`. Each is still passed to the binary as the flag it is, so
# it is not an operand and is not in the table `operands` answers.
REQUIRED_OPTIONS: dict[tuple[str, ...], tuple[str, ...]] = {
    ("task", "content", "set"): ("file",),
    ("task", "create"): ("project", "title"),
    ("document", "create"): ("project", "title"),
}

# The commands that read a body from standard input when `--body-file` is absent, which
# generated methods expose as a `body` the client writes there — a body is never a word of
# the command line, so a caller holding text needs no file to pass it.
BODY_COMMANDS = {
    ("task", "comment", "add"),
    ("task", "comment", "edit"),
    ("task", "create"),
    ("document", "create"),
}

# The commands whose `--answers` generated methods take as a mapping rather than a path: the
# client writes it to the binary's standard input as JSON — which is YAML — and passes
# `--answers -`, so a caller holding answers needs no file to hand them over. A create is in
# both tables, and its method refuses a `body` and `answers` together: standard input holds
# one document.
ANSWERS_COMMANDS = {
    ("template", "render"),
    ("task", "create"),
    ("task", "render"),
    ("document", "create"),
    ("document", "render"),
}

# The commands whose `file` operand names a template, which generated methods check before the
# binary is started: see `_template_file` below. It is optional, because `--template-loader`
# names the template in its place.
TEMPLATE_FILE_COMMANDS = {("template", "variables"), ("template", "render")}

# The repeated options generated methods check are strings before the binary is started: see
# `_strings` below.
TEMPLATE_STRING_LISTS = {"search_path", "var", "unset", "metadata", "repository", "remove_metadata"}


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
        "import json",
        "from collections.abc import Mapping",
        "from typing import Literal",
        "",
        "from pydantic import JsonValue, TypeAdapter",
        "",
        "from .models import (",
        *[
            f"    {root},"
            for root in sorted(
                set(RESPONSE_ROOTS.values())
                | {"GlobalId", "Priority", "SourceName", "StatusCategory"}
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
        "_ANSWERS = TypeAdapter(dict[str, JsonValue])",
        "",
        "",
        "def _answers_document(method: str, answers: Mapping[str, JsonValue]) -> str:",
        '    """The answers as the JSON document the binary reads on standard input.',
        "",
        "    Validated strictly first, so a key that is not a string or a value JSON cannot carry",
        "    is refused here rather than coerced into something the caller did not pass; and",
        "    serialised with `allow_nan=False`, because `json.dumps` would otherwise write a",
        "    non-finite float as a bare `NaN` that the binary reads as text.",
        '    """',
        "    if not isinstance(answers, Mapping):",
        "        kind = type(answers).__name__",
        '        message = f"{method}: answers are a {kind}, not a mapping"',
        "        raise TypeError(message)",
        "    try:",
        "        checked = _ANSWERS.validate_python(dict(answers), strict=True)",
        "        return json.dumps(checked, allow_nan=False)",
        "    except ValueError as error:",
        '        message = f"{method}: answers are not a JSON mapping: {error}"',
        "        raise TypeError(message) from error",
        "",
        "",
        "def _template_file(method: str, file: object) -> str | None:",
        '    """The template path, refused unless it is one — or `None`, for a loader document.',
        "",
        "    Anything but a string would reach the binary as its string form, an empty one names",
        "    no file, and one opening with `-` would be read as an option rather than as the file",
        "    the caller named.",
        '    """',
        "    if file is None:",
        "        return None",
        '    if not isinstance(file, str) or not file or file.startswith("-"):',
        "        message = (",
        '            f"{method}: file is not a template path; next: pass the template\'s "',
        '            "path as a non-empty string, spelling one that starts with `-` as `./-…`"',
        "        )",
        "        raise TypeError(message)",
        "    return file",
        "",
        "",
        "def _strings(",
        "    method: str, option: str, values: object",
        ") -> list[str] | tuple[str, ...] | None:",
        '    """The values of a repeated option, refused unless a list or tuple of strings.',
        "",
        "    Checked rather than passed on whatever they are: each becomes one process argument,",
        "    and anything but a string would reach the binary as its string form — a bare string",
        "    as one argument per character.",
        '    """',
        "    if values is None:",
        "        return None",
        "    if not isinstance(values, (list, tuple)):",
        "        kind = type(values).__name__",
        "        message = (",
        '            f"{method}: {option} is a {kind}, not a list; next: pass a list of "',
        '            "strings"',
        "        )",
        "        raise TypeError(message)",
        "    for index, value in enumerate(values):",
        "        if not isinstance(value, str):",
        "            kind = type(value).__name__",
        "            message = (",
        '                f"{method}: {option}[{index}] is a {kind}, not a string; next: pass "',
        '                "each entry as a string"',
        "            )",
        "            raise TypeError(message)",
        "    return values",
        "",
        "",
        "def _stdin(",
        "    method: str,",
        "    body: str | None,",
        "    answers: Mapping[str, JsonValue] | None,",
        "    named: bool = False,",
        ") -> str | None:",
        '    """What a create or a render writes to the binary\'s standard input.',
        "",
        "    The answers as JSON, or a create's plain body — never both, because standard input",
        "    holds one document; and never a body beside a template, a loader document or a body",
        "    file (`named`), which the binary reads instead of it.",
        '    """',
        "    if body is not None and answers is not None:",
        "        message = (",
        '            f"{method}: body and answers both go to standard input; next: pass a body "',
        '            "without a template, or answers with one"',
        "        )",
        "        raise TypeError(message)",
        "    if body is not None and named:",
        "        message = (",
        '            f"{method}: body is read only when no template, template_loader or "',
        '            "body_file names the body; next: pass one of them, not both"',
        "        )",
        "        raise TypeError(message)",
        "    if body is not None and not isinstance(body, str):",
        "        kind = type(body).__name__",
        '        message = f"{method}: body is a {kind}, not a string"',
        "        raise TypeError(message)",
        "    if answers is not None:",
        "        return _answers_document(method, answers)",
        "    return body",
        "",
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
        # `task priority set` takes the priority it sets as its second operand, spelled as the
        # binary spells it — which is exactly the generated enum's values.
        "priority": "Priority | str",
        # `task content set` reads the task's new content from this file, byte for byte.
        "file": "str",
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
        required = REQUIRED_OPTIONS.get(command, ())
        keywords = [
            item
            for item in option_names(command)
            if item not in taken
            and item not in required
            and not (command in ANSWERS_COMMANDS and item == "answers")
        ]
        body = ["body: str | None = None"] if command in BODY_COMMANDS else []
        if command in ANSWERS_COMMANDS:
            body = [*body, "answers: Mapping[str, JsonValue] | None = None"]
        optional = command in TEMPLATE_FILE_COMMANDS
        parameters = (
            [
                f"{positional}: {positional_types.get(positional, 'str')}"
                + (" | None = None" if optional else "")
                for positional in (*taken, *required)
            ]
            + ["*"]
            + [f"{item}: {option_type(command, item)} | None = None" for item in keywords]
            + body
        )
        if parameters[-1] == "*":
            parameters.pop()
        passed = [f"{item}={item}" for item in [*taken, *required, *keywords]]
        if command in TEMPLATE_FILE_COMMANDS:
            passed[0] = f"file=_template_file({name!r}, file)"
        passed = [
            f"{item}=_strings({name!r}, {item!r}, {item})"
            if item in TEMPLATE_STRING_LISTS
            else entry
            for item, entry in zip([*taken, *required, *keywords], passed, strict=True)
        ]
        if command in ANSWERS_COMMANDS:
            passed.append('answers=None if answers is None else "-"')
            if command in BODY_COMMANDS:
                named = (
                    "template is not None or template_loader is not None or body_file is not None"
                )
                passed.append(f"stdin=_stdin({name!r}, body, answers, {named})")
            else:
                passed.append(f"stdin=_stdin({name!r}, None, answers)")
        elif body:
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
    # After formatting rather than before it: [`enum_defaults`] reads each field in the shape
    # the formatter writes it, which is what lets it be one rule rather than a guess at what
    # the code generator happened to emit on one line or several.
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


# One object's members, each keyed by its wire name, as a schema declares them or a model reads
# them; and every such object of one root, keyed by its members so the two sides can be paired
# without depending on the names the generator chose.
Members = dict[str, JsonValue]
Nullability = dict[frozenset[str], set[frozenset[str]]]


def admits_null(schema: JsonValue, root: JsonValue, seen: frozenset[str] = frozenset()) -> bool:
    """Whether JSON `null` validates against `schema`, following local references.

    A schema is the conjunction of its keywords, so `null` validates only where every keyword
    that can refuse it accepts it; a keyword that constrains another type — `properties`,
    `items`, `format`, `minimum` — accepts `null` by JSON Schema's own rule. A keyword this
    reads in a shape JSON Schema does not define is refused rather than guessed at.
    """
    if isinstance(schema, bool):
        return schema
    if not isinstance(schema, dict):
        raise SystemExit(
            f"binary emitted {json.dumps(schema)} where a schema belongs; next: emit every "
            "member as a JSON Schema object or boolean"
        )
    declared = schema.get("type", "null")
    if not (
        isinstance(declared, str)
        or (isinstance(declared, list) and all(isinstance(kind, str) for kind in declared))
    ):
        raise SystemExit(f"binary emitted a schema `type` of {json.dumps(declared)}")
    enumerated = schema.get("enum", [None])
    if not isinstance(enumerated, list):
        raise SystemExit(f"binary emitted a schema `enum` of {json.dumps(enumerated)}")
    reference = schema.get("$ref")
    if reference is not None and not isinstance(reference, str):
        raise SystemExit(f"binary emitted a schema `$ref` of {json.dumps(reference)}")
    refusals = [
        "null" not in ([declared] if isinstance(declared, str) else declared),
        "const" in schema and schema["const"] is not None,
        None not in enumerated,
        any(not admits_null(v, root, seen) for v in schema_variants(schema, "allOf")),
        "anyOf" in schema
        and not any(admits_null(v, root, seen) for v in schema_variants(schema, "anyOf")),
        "oneOf" in schema
        and sum(admits_null(v, root, seen) for v in schema_variants(schema, "oneOf")) != 1,
        "not" in schema and admits_null(schema["not"], root, seen),
        reference is not None
        and reference not in seen
        and not admits_null(resolve_reference(reference, root), root, seen | {reference}),
    ]
    return not any(refusals)


def schema_variants(schema: dict[str, JsonValue], combinator: str) -> list[JsonValue]:
    """The variants `schema` lists under `combinator`, refusing one that is not a list."""
    variants = schema.get(combinator, [])
    if not isinstance(variants, list):
        raise SystemExit(
            f"binary emitted a schema whose `{combinator}` is not a list; next: emit every "
            "combinator as an array of schemas, as JSON Schema defines it"
        )
    return variants


def resolve_reference(reference: str, root: JsonValue) -> JsonValue:
    """The local definition `reference` names, refusing one the root does not define.

    Refused rather than read as an empty schema, which admits anything: a reference the guard
    cannot follow is a member whose nullability it cannot vouch for.
    """
    definitions = root.get("$defs") if isinstance(root, dict) else None
    name = reference.removeprefix("#/$defs/")
    found = definitions.get(name) if isinstance(definitions, dict) else None
    if name == reference or found is None:
        raise SystemExit(
            f"binary emitted a schema reference {reference!r} that is not a definition of its "
            "own root; next: emit every root with its definitions under `$defs`"
        )
    return found


def schema_objects(value: JsonValue, root: JsonValue, inherited: Members) -> list[Members]:
    """Every object schema under `value` that the generator renders as one model's members.

    An object's own members join those of a definition it references beside them, and an
    object whose `oneOf` or `anyOf` variants carry members of their own is rendered once per
    variant, holding the object's members and that variant's together — which is the shape
    the generator gives an internally tagged enum with fields beside the tag.
    """
    if isinstance(value, list):
        return [found for item in value for found in schema_objects(item, root, {})]
    if not isinstance(value, dict):
        return []
    properties = value.get("properties")
    if "properties" not in value:
        return [found for child in value.values() for found in schema_objects(child, root, {})]
    if not isinstance(properties, dict):
        raise SystemExit(
            f"binary emitted a schema `properties` of {json.dumps(properties)}; next: emit an "
            "object's members as a map from each name to its schema"
        )
    members = dict(inherited)
    reference = value.get("$ref")
    if isinstance(reference, str):
        referenced = resolve_reference(reference, root)
        beside = referenced.get("properties") if isinstance(referenced, dict) else None
        members |= beside if isinstance(beside, dict) else {}
    members |= properties
    found: list[Members] = []
    variants = [
        variant
        for combinator in ("oneOf", "anyOf")
        for variant in schema_variants(value, combinator)
        if isinstance(variant, dict) and isinstance(variant.get("properties"), dict)
    ]
    for variant in variants:
        found += schema_objects(variant, root, members)
    if not variants:
        found.append(members)
    for key, child in value.items():
        if key not in {"oneOf", "anyOf", "properties"}:
            found += schema_objects(child, root, {})
    for child in properties.values():
        found += schema_objects(child, root, {})
    return found


def schema_nullability(schema: JsonValue) -> Nullability:
    """Which members of every object one root declares accept `null`, by the schema."""
    declared: Nullability = {}
    for members in schema_objects(schema, schema, {}):
        nullable = frozenset(
            name for name, member in members.items() if admits_null(member, schema)
        )
        declared.setdefault(frozenset(members), set()).add(nullable)
    return declared


def model_nullability(module: Path) -> Nullability:
    """Which fields of every generated model in `module` accept `None`, by decoding one.

    Decoding rather than reading the annotation's text is the point: it is what a consumer's
    `model_validate` does with an explicit `null`, whatever spelling the generator chose.
    """
    name = f"_nullability_{module.stem}"
    spec = importlib.util.spec_from_file_location(name, module)
    if spec is None or spec.loader is None:
        raise SystemExit(f"cannot load the generated module {module}")
    loaded = importlib.util.module_from_spec(spec)
    # Registered while it runs, because its postponed annotations resolve through it.
    sys.modules[name] = loaded
    try:
        spec.loader.exec_module(loaded)
        generated: Nullability = {}
        for model in vars(loaded).values():
            if not (
                inspect.isclass(model)
                and issubclass(model, BaseModel)
                and not issubclass(model, RootModel)
                and model.__module__ == name
                and model.model_fields
            ):
                continue
            fields = {field.alias or key: field for key, field in model.model_fields.items()}
            nullable = frozenset(
                wire for wire, field in fields.items() if accepts_none(field.annotation)
            )
            generated.setdefault(frozenset(fields), set()).add(nullable)
        return generated
    finally:
        del sys.modules[name]


def accepts_none(annotation: object) -> bool:
    """Whether a field annotated `annotation` decodes an explicit `null`."""
    try:
        TypeAdapter(annotation).validate_python(None)
    except ValidationError:
        return False
    return True


def nullability_disagreements(bundle: SchemaBundle, destination: Path) -> list[str]:
    """Every object whose members accept `null` in the generated models but not the schema.

    Or the other way round, over every root generated into `destination`. A schema object with
    no model holding exactly its members is a disagreement too: a pairing this cannot make is
    a member this cannot vouch for.

    Objects are paired by the names of their members, not by the model names the generator
    chose, so two objects of one root holding the same member names are compared as one set of
    nullability patterns: a pattern present on one side and absent on the other is reported,
    and two such objects trading patterns with each other is not.
    """
    disagreements: list[str] = []
    for root in sorted(set(RESPONSE_ROOTS.values()) | CONTRACT_ROOTS):
        declared = schema_nullability(bundle["roots"][root])
        generated = model_nullability(destination / f"{camel_to_snake(root)}.py")
        for members in sorted(declared.keys() | generated.keys(), key=sorted):
            schema_side = sorted(sorted(nullable) for nullable in declared.get(members, set()))
            model_side = sorted(sorted(nullable) for nullable in generated.get(members, set()))
            if schema_side != model_side:
                disagreements.append(
                    f"{root} object {{{', '.join(sorted(members))}}}: the schema admits null "
                    f"for {schema_side or 'no such object'}, the models for "
                    f"{model_side or 'no such model'}"
                )
    return disagreements


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
        if disagreements := nullability_disagreements(bundle, target):
            raise SystemExit(
                "the generated models and the schema disagree about which members accept "
                "`null`:\n  "
                + "\n  ".join(disagreements)
                + "\nnext: make the schema say what the wire accepts at its Rust source, or "
                "the generator's options generate what the schema says; never edit the "
                "generated text"
            )
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
