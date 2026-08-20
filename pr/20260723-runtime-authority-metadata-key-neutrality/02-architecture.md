# Architecture

## Root abstraction defect

The SDK exposes generic authority metadata concepts, but the wire keys still include the EasyNet product name. That makes the canonical runtime model look product-owned and pressures downstream products to preserve EasyNet naming even when the abstraction is generic.

## Clean target

Authority metadata keys are SDK-owned runtime keys:

- `x-runtime-delegation`
- `x-runtime-session-authority`

The daemon admission path consumes those canonical keys. Product-specific hosted-agent delegation metadata remains in the EasyNet daemon provider domain and is intentionally out of scope for this slice.
