"""Subprocess client for the onetaskgraph command."""

from __future__ import annotations

import os
import shutil
import subprocess
from collections.abc import Mapping
from pathlib import Path
from typing import TYPE_CHECKING, cast

from pydantic import RootModel, TypeAdapter, ValidationError

from ._generated import GeneratedClient
from ._generated.client import POSITIONALS

if TYPE_CHECKING:
    from ._generated.failure_document import Failure, FailureDocument


REPORTING_EXITS: dict[tuple[str, ...], frozenset[int]] = {
    ("sources", "sanitize"): frozenset({1, 3}),
    # A check answers its record on exit 3 when it is not valid — void or absent.
    **{(noun, "attestation", "check"): frozenset({3}) for noun in ("task", "project", "document")},
}
"""The exits beyond 0 and 4 on which a command still answers its own document."""


class OnetaskgraphError(RuntimeError):
    """The command could not produce a response."""

    def __init__(
        self,
        message: str,
        *,
        exit_code: int,
        failure: Failure | None = None,
        document: FailureDocument | None = None,
        answer: object = None,
    ) -> None:
        """Remember the diagnostic, the stable process status and the binary's whole answer.

        ``failure`` is the failure the binary wrote under ``--json`` when it refused the
        command, so a caller branches on its ``kind`` rather than on the message's wording;
        ``document`` is the whole failure document it is the member of — what the item held,
        what an expected write came to — and ``answer`` that document as the JSON it was
        written, so nothing the store answered is discarded. Each is ``None`` when the process
        wrote none.
        """
        super().__init__(message)
        self.exit_code = exit_code
        self.failure = failure if failure is not None else getattr(document, "failure", None)
        self.document = document
        self.answer = answer

    @property
    def kind(self) -> str | None:
        """The failure's ``kind`` — ``version-moved``, ``no-such-item`` and so on."""
        return None if self.failure is None else self.failure.kind


class VersionMovedError(OnetaskgraphError):
    """A write held to a version (``if_version``) found the item at another, and wrote nothing."""

    @property
    def current_version(self) -> str | None:
        """The version the item is at now, as the store's refusal named it."""
        held = None if self.document is None else self.document.held
        return None if held is None else held.version


class PartialWriteError(OnetaskgraphError):
    """A write that landed in part (exit 4), raised only by a client opted in to it.

    ``result`` is the command's answer parsed into its typed model — for a copy, the
    ``CopyReport`` with every item's outcome, the ones that landed and the ones that failed —
    and ``answer`` the same answer as the JSON the store wrote. A client constructed without
    ``raise_on_partial`` returns ``result`` instead, as it always has. Which commands those
    are is `PARTIAL_WRITE_COMMANDS`.
    """

    def __init__(self, message: str, *, exit_code: int, result: object, answer: object) -> None:
        """Keep the typed answer beside its JSON."""
        super().__init__(message, exit_code=exit_code, answer=answer)
        self.result = result


# Every command of the binary, by whether its exit 4 means a write landed in part — the ones
# `raise_on_partial` raises for — or not: a read whose exit 4 is a partial answer, or a command
# that writes nothing of anybody's work. Both sets are written out whole, and
# tests/test_followup.py reconciles their union against the commands the binary's own schema
# bundle lists, so a command added there is refused until it is put in one of them.
PARTIAL_WRITE_COMMANDS: frozenset[tuple[str, ...]] = frozenset(
    {
        ("task", "copy"),
        ("project", "copy"),
        ("document", "copy"),
        ("task", "create"),
        ("project", "create"),
        ("document", "create"),
        ("task", "render"),
        ("project", "render"),
        ("document", "render"),
        ("task", "update"),
        ("task", "status", "set"),
        ("project", "status", "set"),
        ("task", "status", "reconcile"),
        ("task", "status", "complete"),
        ("task", "priority", "set"),
        ("task", "assignee", "set"),
        ("task", "content", "set"),
        ("task", "metadata", "set"),
        ("project", "metadata", "set"),
        ("document", "metadata", "set"),
        # An attestation is a metadata write of its reserved key, and fails as one does.
        ("task", "attest"),
        ("project", "attest"),
        ("document", "attest"),
        ("task", "comment", "add"),
        ("task", "comment", "edit"),
        ("task", "comment", "delete"),
        # A batch answers each item on its own, and exits 4 when any was not written.
        ("task", "comment", "add-many"),
        ("task", "update-many"),
        ("task", "parent", "set-many"),
        ("task", "link", "add-many"),
        ("task", "link", "remove-many"),
        ("task", "project-human"),
        ("sources", "fields"),
        ("sources", "status-options"),
        ("sources", "sanitize"),
    }
)
NOT_PARTIAL_WRITE_COMMANDS: frozenset[tuple[str, ...]] = frozenset(
    {
        ("schema",),
        ("config", "show"),
        ("sources", "list"),
        ("sources", "route"),
        ("task", "list"),
        ("task", "show"),
        ("task", "show-many"),
        ("task", "relations"),
        ("task", "deps"),
        ("task", "locate"),
        ("task", "comment", "list"),
        ("task", "comments"),
        # A read: exit 4 says a search's pages were cut short, never that anything was written.
        ("task", "find"),
        ("task", "answers"),
        ("task", "assets"),
        ("project", "list"),
        ("project", "show"),
        ("project", "deps"),
        ("project", "graph"),
        ("project", "answers"),
        ("document", "list"),
        ("document", "show"),
        ("document", "answers"),
        # A read: exit 3 says the record is not valid, never that anything was written.
        ("task", "attestation", "check"),
        ("project", "attestation", "check"),
        ("document", "attestation", "check"),
        ("label", "list"),
        ("search",),
        # Empties this host's cache of a source, never anything of the source's own work.
        ("cache", "clear"),
        ("template", "variables"),
        ("template", "render"),
    }
)


class Client(GeneratedClient):
    """Typed client that invokes the real binary once per method call."""

    def __init__(
        self,
        binary: str | Path | None = None,
        *,
        cwd: str | Path | None = None,
        environment: Mapping[str, str] | None = None,
        raise_on_partial: bool = False,
    ) -> None:
        """Resolve the binary and establish the invocation directory.

        With ``raise_on_partial``, a write that lands in part (exit 4) raises
        `PartialWriteError` carrying its typed answer and its JSON; without it, the default,
        that answer is returned, as it always has been.
        """
        self.binary = _resolve_binary(binary, environment)
        self.raise_on_partial = raise_on_partial
        self.cwd = Path(cwd) if cwd is not None else None
        self.environment = dict(environment if environment is not None else os.environ)
        self.environment.pop("ONETASKGRAPH_SDK_BINARY", None)

    async def _invoke[T](
        self,
        command: list[str],
        model: object,
        *,
        stdin: str | None = None,
        **options: object,
    ) -> T:
        arguments = [self.binary, *command]
        # Read from the generated table rather than matched again here: the generated
        # methods and this argument vector have to name the same operands, and while each
        # kept its own copy a verb added to one and forgotten in the other produced a
        # method that could not do what it was named for.
        for positional in POSITIONALS.get(tuple(command), ()):
            try:
                value = options.pop(positional)
            except KeyError as error:
                raise TypeError(f"missing required argument: {positional}") from error
            # An operand a flag can stand in for — a template's file, beside
            # `template_loader` — is left out rather than passed as the word `None`.
            if value is None:
                continue
            # A copy verb's positional is variadic; every other takes exactly one, so a
            # bare value is passed through as a list of one.
            given = value if isinstance(value, (list, tuple)) else [value]
            arguments.extend(
                str(item.root if isinstance(item, RootModel) else item) for item in given
            )
        for name, value in options.items():
            flag = f"--{name.removesuffix('_').replace('_', '-')}"
            # A term scope is a list of repositories, or an empty list for none at all — which
            # the binary spells as a flag of its own — or `None` for every one the store's
            # policy knows of. It reaches the binary unchanged either way.
            if name == "term_scope" and isinstance(value, (list, tuple)) and not value:
                arguments.append("--term-scope-empty")
                continue
            match value:
                case None | False:
                    continue
                case True:
                    arguments.append(flag)
                case list() | tuple():
                    for item in value:
                        # A repeated id option takes a `GlobalId` as its positional does.
                        text = item.root if isinstance(item, RootModel) else item
                        arguments.extend((flag, str(text)))
                case RootModel():
                    # A single id option — `origin` — takes a `GlobalId` as a positional does.
                    arguments.extend((flag, str(value.root)))
                case _:
                    arguments.extend((flag, str(value)))
        # Every call is non-interactive: a library caller has no terminal to be asked on, and a
        # command that would prompt refuses what it was not given instead of waiting.
        arguments.extend(("--json", "--no-interactive"))
        completed = await self._invoke_process(arguments, stdin)
        # A sweep answers its report on exit 1 (something left unfixable or in conflict) and
        # exit 3 (coverage incomplete): the report says which. Any other document there — a
        # refusal's failure document — is not that report, and fails below as a refusal.
        if (
            tuple(command) in REPORTING_EXITS
            and completed.returncode in REPORTING_EXITS[tuple(command)]
        ):
            try:
                # As below: TypeAdapter validates `model`; its dynamic constructor cannot
                # preserve T.
                return cast(T, TypeAdapter(model).validate_json(completed.stdout))
            except ValidationError:
                pass
        if completed.returncode not in {0, 4}:
            document = _failure_document_of(completed.stdout)
            error = (
                VersionMovedError
                if document is not None and document.failure.kind == "version-moved"
                else OnetaskgraphError
            )
            raise error(
                completed.stderr.strip(),
                exit_code=completed.returncode,
                document=document,
                answer=_json_of(completed.stdout),
            )
        try:
            # TypeAdapter validates `model`; its dynamic constructor cannot preserve T.
            result = cast(T, TypeAdapter(model).validate_json(completed.stdout))
        except ValidationError as error:
            raise OnetaskgraphError(
                f"binary returned a response outside its emitted schema: {error}",
                exit_code=completed.returncode,
            ) from error
        if (
            completed.returncode == 4
            and self.raise_on_partial
            and tuple(command) in PARTIAL_WRITE_COMMANDS
        ):
            raise PartialWriteError(
                completed.stderr.strip() or "the write landed in part",
                exit_code=completed.returncode,
                result=result,
                answer=_json_of(completed.stdout),
            )
        return result

    async def _invoke_process(
        self, arguments: list[str], stdin: str | None = None
    ) -> subprocess.CompletedProcess[str]:
        """Cross the process boundary without blocking the event-loop IO layer.

        Standard input is always a pipe, written with `stdin` and closed: a command that reads
        a body there must never block on the caller's own terminal, and one given no body reads
        an empty one and says so rather than waiting.
        """
        # Imported here rather than with the module: a caller that only validates a document
        # the binary printed never starts a process, and asyncio is a large share of the
        # import a short-lived caller would otherwise pay for nothing.
        import asyncio

        process = await asyncio.create_subprocess_exec(
            *arguments,
            cwd=self.cwd,
            env=self.environment,
            stdin=asyncio.subprocess.PIPE,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        stdout, stderr = await process.communicate((stdin or "").encode())
        return subprocess.CompletedProcess(
            arguments, process.returncode or 0, stdout.decode(), stderr.decode()
        )


def _failure_document_of(stdout: str) -> FailureDocument | None:
    """The failure document a refused command wrote, when its schema describes what it wrote."""
    from ._generated.failure_document import FailureDocument

    try:
        return FailureDocument.model_validate_json(stdout)
    except ValidationError:
        return None


def _json_of(stdout: str) -> object:
    """What a refused command wrote, as JSON, or ``None`` when it wrote no JSON."""
    import json

    try:
        return json.loads(stdout)
    except ValueError:
        return None


def _resolve_binary(explicit: str | Path | None, environment: Mapping[str, str] | None) -> str:
    """Resolve explicit, environment, then distribution-provided executable."""
    if explicit is not None:
        candidate = str(explicit)
    else:
        values = environment if environment is not None else os.environ
        candidate = values.get("ONETASKGRAPH_SDK_BINARY", "")
        if not candidate:
            candidate = shutil.which("onetaskgraph", path=values.get("PATH")) or ""
    if not candidate:
        raise FileNotFoundError(
            "onetaskgraph binary not found; pass binary=, set ONETASKGRAPH_SDK_BINARY, "
            "or install the binary distribution"
        )
    path = Path(candidate)
    if not path.is_file() or not os.access(path, os.X_OK):
        raise FileNotFoundError(f"onetaskgraph binary is not an executable file: {candidate}")
    return str(path.resolve())
