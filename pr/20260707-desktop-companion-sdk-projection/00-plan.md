# Desktop Companion SDK Projection Plan

## Intent

Expose the desktop companion shared DTO contract through EasyNet-Cli SDK facades without moving lifecycle ownership into Axon SDKs or product-specific downstream code.

## Scope

- Add Python DTOs for companion status, list, and lifecycle action result.
- Add Python daemon-handle lifecycle methods backed by the C ABI companion functions.
- Add Go DTOs and daemon-handle lifecycle methods behind an optional companion transport seam.
- Keep Swift and Java companion projections as remaining work for the next SDK slice.

## Non-Scope

- No Axon Invocation primitive changes.
- No remote companion control exposure.
- No OS supervisor behavior changes.
