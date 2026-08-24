"""Subprocess client for the onetaskgraph command."""

from __future__ import annotations

import os
import shutil
import subprocess
from collections.abc import Mapping
from pathlib import Path
from typing import cast

from pydantic import TypeAdapter, ValidationError

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
        self.environment = dict(environment) if environment is not None else None
        if self.environment is not None:
            self.environment.pop("ONETASKGRAPH_SDK_BINARY", None)

    def _invoke[T](self, command: list[str], model: object, **options: object) -> T:
        arguments = [self.binary, *command]
        positional = (
            "text"
            if command == ["search"]
            else "id"
            if command[0] in {"task", "project"} and command[-1] in {"show", "deps"}
            else None
        )
        if positional is not None:
            try:
                arguments.append(str(options.pop(positional)))
            except KeyError as error:
                raise TypeError(f"missing required argument: {positional}") from error
        for name, value in options.items():
            if value is None or value is False:
                continue
            flag = f"--{name.replace('_', '-')}"
            if value is True:
                arguments.append(flag)
            elif isinstance(value, (list, tuple)):
                for item in value:
                    arguments.extend((flag, str(item)))
            else:
                arguments.extend((flag, str(value)))
        arguments.append("--json")
        completed = subprocess.run(
            arguments,
            cwd=self.cwd,
            env=self.environment,
            text=True,
            capture_output=True,
            check=False,
        )
        if completed.returncode not in {0, 4}:
            raise OnetaskgraphError(completed.stderr.strip(), exit_code=completed.returncode)
        try:
            return cast(T, TypeAdapter(model).validate_json(completed.stdout))
        except ValidationError as error:
            raise OnetaskgraphError(
                f"binary returned a response outside its emitted schema: {error}",
                exit_code=completed.returncode,
            ) from error


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
    return candidate
