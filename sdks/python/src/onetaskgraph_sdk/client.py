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

    async def _invoke[T](self, command: list[str], model: object, **options: object) -> T:
        arguments = [self.binary, *command]
        match command:
            case ["search"]:
                positional = "text"
            case ["task" | "project", "show" | "deps"]:
                positional = "id"
            case _:
                positional = None
        if positional is not None:
            try:
                value = options.pop(positional)
                arguments.append(str(value.root if isinstance(value, RootModel) else value))
            except KeyError as error:
                raise TypeError(f"missing required argument: {positional}") from error
        for name, value in options.items():
            flag = f"--{name.removesuffix('_').replace('_', '-')}"
            match value:
                case None | False:
                    continue
                case True:
                    arguments.append(flag)
                case list() | tuple():
                    for item in value:
                        arguments.extend((flag, str(item)))
                case _:
                    arguments.extend((flag, str(value)))
        arguments.append("--json")
        completed = await self._invoke_process(arguments)
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

    async def _invoke_process(self, arguments: list[str]) -> subprocess.CompletedProcess[str]:
        """Cross the process boundary without blocking the event-loop IO layer."""
        process = await asyncio.create_subprocess_exec(
            *arguments,
            cwd=self.cwd,
            env=self.environment,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        stdout, stderr = await process.communicate()
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
