# Descriptor resolution typed owner-offline

Date: 2026-07-29

## Goal

Remove message-string classification from the descriptor resolution provider's
owner-offline outcome. Descriptor resolution must return a typed owner-offline
state before SDK/FFI projection so product callers can distinguish "owner is
not online" from "ability does not exist".

## Invariants

1. FFI remains an ABI/DTO bridge: it may project a typed provider error, but it
   must not classify descriptor business outcomes from message substrings.
2. Runtime descriptor resolution exposes the hardening result set:
   `InvalidRequest`, `RuntimeOwnerUnavailable`, `DescriptorNotFound`,
   `OwnerOffline`, `OwnerMismatch`, and `CallModeUnsupported`.
3. Owner offline is retry-safe route unavailability, not ability absence.
4. Local catalog miss remains non-retryable descriptor not found.
5. Existing public SDK error compatibility is preserved through typed
   projection, not through legacy fallback logic.

## Boundary decision

The descriptor provider owns the business state. FFI maps provider states to
ABI error codes and stable JSON projection only. Runtime failure text parsing is
allowed as legacy external error normalization, but descriptor resolution must
not rely on it for provider-owned states.

## Verification plan

- Add provider/FFI tests proving `OwnerOffline` projects to
  `DESCRIPTOR_OWNER_OFFLINE`.
- Add a negative source gate preventing descriptor resolver projection from
  using `RuntimeFailureKind::DescriptorOwnerOffline`.
- Run Rust fmt and targeted descriptor tests.
- Run canonical runtime convergence and SDK public API gates.
