"""Validation of JSON read from outside the process: files, answers and subprocess output.

Every reader in this package turns an untrusted value into a typed one through these helpers,
which raise `ShapeError` naming the path of the first value that is not what was expected,
rather than letting a `KeyError` or a wrong type travel into arithmetic.
"""

from __future__ import annotations

import dataclasses
import json
import types
import typing
from enum import Enum
from pathlib import Path

type Json = None | bool | int | float | str | list[Json] | dict[str, Json]


class ShapeError(ValueError):
    """A JSON value is not the shape its reader requires."""


def load(path: Path) -> Json:
    """Parse one JSON file, naming the file when it does not parse."""
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as error:
        raise ShapeError(f"{path}: {error}") from error


def load_lines(path: Path) -> list[Json]:
    """Parse a JSON-lines file; an absent file holds no lines."""
    if not path.exists():
        return []
    out: list[Json] = []
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if line.strip():
            try:
                out.append(json.loads(line))
            except ValueError as error:
                raise ShapeError(f"{path}:{number}: {error}") from error
    return out


def parse(text: str, where: str) -> Json:
    """Parse JSON text from `where`, naming it when the text does not parse."""
    try:
        return json.loads(text)
    except ValueError as error:
        raise ShapeError(f"{where}: {error}") from error


def obj(value: Json, where: str) -> dict[str, Json]:
    """`value` as an object."""
    if not isinstance(value, dict):
        raise ShapeError(f"{where}: expected an object, found {type(value).__name__}")
    return value


def arr(value: Json, where: str) -> list[Json]:
    """`value` as an array."""
    if not isinstance(value, list):
        raise ShapeError(f"{where}: expected an array, found {type(value).__name__}")
    return value


def text(value: Json, where: str) -> str:
    """`value` as a string."""
    if not isinstance(value, str):
        raise ShapeError(f"{where}: expected a string, found {type(value).__name__}")
    return value


def integer(value: Json, where: str) -> int:
    """`value` as an integer (a boolean is not one)."""
    if isinstance(value, bool) or not isinstance(value, int):
        raise ShapeError(f"{where}: expected an integer, found {type(value).__name__}")
    return value


def number(value: Json, where: str) -> float:
    """`value` as a number (a boolean is not one)."""
    if isinstance(value, bool) or not isinstance(value, int | float):
        raise ShapeError(f"{where}: expected a number, found {type(value).__name__}")
    return float(value)


def flag(value: Json, where: str) -> bool:
    """`value` as a boolean."""
    if not isinstance(value, bool):
        raise ShapeError(f"{where}: expected a boolean, found {type(value).__name__}")
    return value


def field(mapping: dict[str, Json], key: str, where: str) -> Json:
    """`mapping[key]`, naming the missing key."""
    if key not in mapping:
        raise ShapeError(f"{where}: missing `{key}`")
    return mapping[key]


def optional_int(value: Json, where: str) -> int | None:
    """`value` as an integer, or `None`."""
    return None if value is None else integer(value, where)


def encode(value: object) -> Json:
    """A dataclass tree as JSON: enums by value, tuples as arrays, `None` kept."""
    match value:
        case Enum():
            return encode(value.value)
        case _ if dataclasses.is_dataclass(value) and not isinstance(value, type):
            return {f.name: encode(getattr(value, f.name)) for f in dataclasses.fields(value)}
        case dict():
            return {str(encode(k)): encode(v) for k, v in value.items()}
        case tuple() | list():
            return [encode(v) for v in value]
        case None | bool() | int() | float() | str():
            return value
        case _:
            raise ShapeError(f"cannot encode a {type(value).__name__}")


def decode[T](kind: type[T], value: Json, where: str) -> T:
    """`value` as an instance of `kind`, every field validated against its annotation."""
    result = _decode(kind, value, where)
    if not isinstance(result, kind):
        raise ShapeError(f"{where}: decoded a {type(result).__name__}, not a {kind.__name__}")
    return result


def _decode(kind: object, value: Json, where: str) -> object:
    supertype = getattr(kind, "__supertype__", None)
    if supertype is not None:
        return _decode(supertype, value, where)
    origin, args = typing.get_origin(kind), typing.get_args(kind)
    if origin in (types.UnionType, typing.Union):
        if value is None and type(None) in args:
            return None
        inner = [a for a in args if a is not type(None)]
        return _decode(inner[0], value, where)
    if origin is tuple:
        items = arr(value, where)
        if len(args) == 2 and args[1] is Ellipsis:
            return tuple(_decode(args[0], v, f"{where}[{i}]") for i, v in enumerate(items))
        if len(items) != len(args):
            raise ShapeError(f"{where}: expected {len(args)} items, found {len(items)}")
        return tuple(
            _decode(a, v, f"{where}[{i}]") for i, (a, v) in enumerate(zip(args, items, strict=True))
        )
    if origin is dict:
        mapping = obj(value, where)
        return {
            _decode(args[0], parse_key(args[0], k), where): _decode(args[1], v, f"{where}.{k}")
            for k, v in mapping.items()
        }
    if isinstance(kind, type) and issubclass(kind, Enum):
        try:
            return kind(value)
        except ValueError as error:
            raise ShapeError(f"{where}: {error}") from error
    if isinstance(kind, type) and dataclasses.is_dataclass(kind):
        mapping = obj(value, where)
        hints = typing.get_type_hints(kind)
        return kind(
            **{
                f.name: _decode(hints[f.name], field(mapping, f.name, where), f"{where}.{f.name}")
                for f in dataclasses.fields(kind)
            }
        )
    match kind:
        case type() if kind is bool:
            return flag(value, where)
        case type() if kind is int:
            return integer(value, where)
        case type() if kind is float:
            return number(value, where)
        case type() if kind is str:
            return text(value, where)
        case _:
            raise ShapeError(f"{where}: no decoder for {kind!r}")


def parse_key(kind: object, key: str) -> Json:
    """A JSON object key as the value its declared key type decodes from."""
    if isinstance(kind, type) and issubclass(kind, int) and not issubclass(kind, bool):
        return int(key)
    return key
