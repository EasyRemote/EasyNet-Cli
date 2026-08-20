# Intent

## Goal

Add an SDK-owned Go projection seam for daemon directory read-model enum values. Product repositories should normalize node state and trust-level fields through EasyNet-Cli SDK, not by importing Axon SDK enum helpers or generated protobuf packages directly.

## Non-goals

- Do not edit `docs/spec/daemon-sdk-requirements-v1.md`.
- Do not add backend-specific DTOs to the SDK.
- Do not introduce compatibility paths for old backend raw Axon imports.

## Acceptance Criteria

- Go SDK exposes Directory/Identity profile helpers for read-model enum projection.
- Helpers preserve existing product-visible string behavior for numeric and string wire shapes.
- Focused Go SDK tests cover known, unknown, nil, string, and unexpected enum values.
