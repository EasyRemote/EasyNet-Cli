# Intent

## Goal

Move advertised AbilityDescriptor read-model projection into the SDK canonical runtime model for both Go and Python. Product repositories should not import raw Axon SDK helpers to read descriptor maps.

## Non-goals

- Do not edit `docs/spec/daemon-sdk-requirements-v1.md`.
- Do not add EasyNet backend display DTOs to the SDK.
- Do not make the Go SDK import Axon SDK or generated protobuf packages.

## Acceptance Criteria

- Go SDK exposes a generic `AbilityDescriptorProjection` with protocol/runtime fields and opaque metadata.
- Python SDK exposes the same projection concept.
- Projection accepts bare publisher descriptor maps and resolver-summary maps with nested `descriptor`.
- Focused SDK tests cover nested override, namespace/local_name name join, hints, schema input, and metadata.
