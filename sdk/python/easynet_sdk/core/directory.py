"""Provider-neutral Directory core model."""

from __future__ import annotations

from enum import StrEnum


class DirectoryResolveKind(StrEnum):
    ROUTE = "RESOLVE_TYPE_ROUTE"
    DIRECTORY_LISTING = "RESOLVE_TYPE_DIRECTORY_LISTING"
    CANONICAL_IDENTITY = "RESOLVE_TYPE_CANONICAL_IDENTITY"
    OWNER = "RESOLVE_TYPE_OWNER"
