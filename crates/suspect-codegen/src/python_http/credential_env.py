"""Creation-time environment defaults for an explicitly configured source policy."""
from __future__ import annotations

from collections.abc import Mapping
from typing import final
from ._types import Credential


@final
class _OmittedAuth:
    __slots__ = ()

    def __repr__(self) -> str:
        return '<auth omitted>'


_OMITTED_AUTH = _OmittedAuth()


def _credentials(auth: Mapping[str, Credential] | None | _OmittedAuth) -> Mapping[str, Credential] | None:
    if not isinstance(auth, _OmittedAuth):
        return auth
    # Explicit arguments return above without accessing the process environment.
    # Importing the generated package defines this function and never calls it.
    try:
        import os
    except ImportError:
        return {}
    values: dict[str, str | None] = {}
    credentials: dict[str, Credential] = {}
    for name, variable in _BINDINGS:
        if variable not in values:
            try:
                value = os.getenv(variable)
            except Exception:
                # Environment access may be unavailable. Retain no exception or
                # value; ordinary operation security handles missing credentials.
                value = None
            values[variable] = value if type(value) is str and value else None
        supplied = values[variable]
        if supplied is not None:
            credentials[name] = supplied
    return credentials
