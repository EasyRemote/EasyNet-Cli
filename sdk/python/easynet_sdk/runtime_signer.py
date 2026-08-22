"""Local runtime signer selection over daemon-managed key custody."""

from __future__ import annotations

from collections.abc import Callable
from typing import NoReturn, Protocol

from .errors import ErrorCode, RetryHint, SDKError
from .managed_signing import ManagedSigner, ManagedSigningClient
from .signer_handle import SignerHandle, signer_handle_provenance_error
from .signing import Signer

USER_RUNTIME_SIGNING_PURPOSE = "user_signing.cli"


class RuntimeSignerProvider(Protocol):
    """Resolve the daemon-custodied signer selected for one local caller."""

    def resolve(
        self,
        caller_ura: str,
        requested: Signer | None = None,
    ) -> Signer: ...


class LocalRuntimeSignerProvider:
    """Select only the active key-service signer for a local runtime caller.

    A caller-supplied signer is an exact key-selection pin. It never supplies
    signing custody: the returned signer is always reconstructed from the
    active managed key selected by the local key service.
    """

    def __init__(
        self,
        *,
        key_service_path: str = "",
        managed_signer_loader: Callable[[str], ManagedSigner] | None = None,
    ) -> None:
        self._managed_signer_loader = managed_signer_loader or (
            lambda caller_ura: ManagedSigningClient(
                key_service_path
            ).active_signer_for_subject(
                caller_ura,
                purpose=USER_RUNTIME_SIGNING_PURPOSE,
            )
        )

    def resolve(
        self,
        caller_ura: str,
        requested: Signer | None = None,
    ) -> Signer:
        caller = _required_caller(caller_ura)
        active = self._active_signer(caller)
        if requested is not None:
            pinned = _canonical_managed_signer(requested, caller)
            if not _same_key_selection(active.handle, pinned.handle):
                _signer_unavailable(
                    "requested signer is not the active managed signer for caller"
                )
        return active

    def _active_signer(self, caller_ura: str) -> Signer:
        try:
            managed = self._managed_signer_loader(caller_ura)
            if not isinstance(managed, ManagedSigner):
                _signer_unavailable(
                    "local signer loader did not return a managed key-service signer"
                )
            signer = managed.invocation_signer()
        except SDKError as exc:
            if exc.stage == "runtime_signer":
                raise
            _signer_unavailable(
                f"active managed signer is unavailable for {caller_ura}: {exc}",
                cause=exc,
            )
        _validate_handle(signer.handle, caller_ura)
        return signer


def _canonical_managed_signer(requested: Signer, caller_ura: str) -> Signer:
    if not isinstance(requested, Signer) or not isinstance(
        requested.provider, ManagedSigner
    ):
        _signer_unavailable(
            "requested signer must be backed by the managed key service"
        )
    try:
        canonical = requested.provider.invocation_signer()
    except SDKError as exc:
        _signer_unavailable(
            f"requested managed signer is not usable: {exc}",
            cause=exc,
        )
    if requested.handle != canonical.handle:
        _signer_unavailable(
            "requested signer handle does not match its managed key-service provider"
        )
    _validate_handle(canonical.handle, caller_ura)
    return canonical


def _validate_handle(handle: SignerHandle, caller_ura: str) -> None:
    provenance_error = signer_handle_provenance_error(handle)
    if provenance_error:
        _signer_unavailable(provenance_error)
    if handle.owner_ura != caller_ura:
        _signer_unavailable("managed signer owner does not match caller URA")


def _same_key_selection(active: SignerHandle, requested: SignerHandle) -> bool:
    return (
        active.owner_ura == requested.owner_ura
        and active.key_id == requested.key_id
        and active.signer_id == requested.signer_id
        and active.algorithm == requested.algorithm
        and active.policy.get("policy_ref") == requested.policy.get("policy_ref")
    )


def _required_caller(value: str) -> str:
    if not isinstance(value, str) or not value.strip() or value != value.strip():
        _signer_unavailable("caller URA is required and must already be trimmed")
    return value


def _signer_unavailable(
    message: str,
    *,
    cause: BaseException | None = None,
) -> NoReturn:
    raise SDKError(
        code=ErrorCode.CALLER_SIGNER_UNAVAILABLE,
        stage="runtime_signer",
        retry=RetryHint.NEVER,
        retryable=False,
        message=message,
        cause=cause,
    )
