# Architecture

## Root Abstraction Problem

`InvocationDraft` is the complete seven-tuple snapshot. If `descriptor_ref` is merely a non-empty string, downstream runtime paths can receive calls that are not bound to a descriptor version.

## Target Architecture

- Validate descriptor-ref shape during draft inspection.
- Keep full canonical construction in Directory + Identity helpers.
- Use a product-neutral Python value object for the current seam and the existing Go Axon-delegated parser where available.

## Module Boundaries

- `sdk/go/ability_descriptor.go`: Go descriptor-ref parser facade.
- `sdk/go/invocation.go`: Go draft construction.
- `sdk/python/easynet_sdk/ability_descriptor.py`: Python descriptor-ref value object seam.
- `sdk/python/easynet_sdk/invocation.py`: Python draft construction.
