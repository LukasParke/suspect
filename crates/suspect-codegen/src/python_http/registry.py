"""The generated native bindings accompany, and never reinterpret, the protocol plan."""
from __future__ import annotations
import functools
import json
from pathlib import Path
from typing import Any, cast
from . import json_runtime as J, model_codecs
from .codec_runtime import ModelCodec
from ._types import Source
from ._wire import location


@functools.lru_cache(maxsize=1)
def plan() -> dict[str, Any]:
    return cast(dict[str, Any], json.loads(Path(__file__).with_name('protocol-plan.json').read_text(encoding='utf-8'), parse_float=J.JsonNumber))


@functools.lru_cache(maxsize=1)
def codecs() -> dict[Source, ModelCodec[Any]]:
    return {location(entry['source']): getattr(model_codecs, entry['name'] + 'Codec') for entry in plan()['models']}


def codec(reference: dict[str, Any]) -> ModelCodec[Any]:
    return codecs()[location(reference['schema']['id'])]


def native_class(name: str) -> type[Any]:
    from . import operations
    return cast(type[Any], getattr(operations, name))


class Operation:
    def __init__(self, index: int) -> None:
        self.wire: dict[str, Any] = plan()['protocol']['operations'][index]
        self.binding: dict[str, Any] = plan()['bindings'][index]
        self.source = location(self.binding['source'])
        self.max_request_bytes: int = plan()['limits']['request']
        self.max_response_bytes: int = plan()['limits']['response']
        self.max_part_bytes: int = plan()['limits']['part']
        self.max_parts: int = plan()['limits']['parts']


@functools.lru_cache(maxsize=None)
def operation(index: int) -> Operation:
    return Operation(index)
