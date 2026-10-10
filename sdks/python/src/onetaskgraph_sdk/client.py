"""Subprocess client for the onetaskgraph command."""

from __future__ import annotations

import asyncio
import os
import shutil
import subprocess
from collections.abc import Mapping
from pathlib import Path
from typing import cast

from pydantic import RootModel, TypeAdapter, ValidationError

from ._generated import GeneratedClient
from ._generated.client import POSITIONALS


class OnetaskgraphError(RuntimeError):
    """The command could not produce a response."""

    def __init__(self, message: str, *, exit_code: int) -> None:
        """Remember both the diagnostic and stable process status."""
        super().__init__(message)
        self.exit_code = exit_code


class Client(GeneratedClient):
    """Typed client that invokes the real binary once per method call."""

    def __init__(
        self,
        binary: str | Path | None = None,
        *,
        cwd: str | Path | None = None,
        environment: Mapping[str, str] | None = None,
    ) -> None:
        """Resolve the binary and establish the invocation directory."""
        self.binary = _resolve_binary(binary, environment)
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
        if completed.returncode not in {0, 4}:
            raise OnetaskgraphError(completed.stderr.strip(), exit_code=completed.returncode)
        try:
            # TypeAdapter validates `model`; its dynamic constructor cannot preserve T.
            return cast(T, TypeAdapter(model).validate_json(completed.stdout))
        except ValidationError as error:
            raise OnetaskgraphError(
                f"binary returned a response outside its emitted schema: {error}",
                exit_code=completed.returncode,
            ) from error

    async def _invoke_process(
        self, arguments: list[str], stdin: str | None = None
    ) -> subprocess.CompletedProcess[str]:
        """Cross the process boundary without blocking the event-loop IO layer.

        Standard input is always a pipe, written with `stdin` and closed: a command that reads
        a body there must never block on the caller's own terminal, and one given no body reads
        an empty one and says so rather than waiting.
        """
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
