# Architecture

## Layering

1. Core runtime owns admission, authority, descriptor, route, and terminal lifecycle semantics.
2. SDKs expose generic runtime concepts and conformance guards.
3. CLI/product facades adapt UX commands to canonical runtime calls only.

## Boundary under review

The active investigation targets product-visible runtime ingress paths that still encode legacy authority or compatibility behavior outside the canonical runtime owner.

## Expected module ownership

- `src/daemon/invocation/**`: canonical admission and authority validation.
- `src/support/platform/local_invoke.rs`: local IPC ingress adapters and named local issuers.
- `src/cli/**`: UX projection only; no second authority, descriptor, or lifecycle model.
- `sdk/**`: product-neutral canonical runtime model only.
