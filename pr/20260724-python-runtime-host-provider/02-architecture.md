# Architecture

## Root abstraction problem

Python SDK lifecycle and transport providers still live under `providers.easynet`, and identity.py maps EasyNet-specific credential fields into the canonical runtime identity projection. This keeps product deployment and directory concepts inside the SDK provider namespace.

## Target model

- `providers.runtime.lifecycle` owns runtime host lifecycle request DTOs.
- `providers.runtime.transport` owns C ABI and direct runtime invocation transport lowering.
- Runtime identity projection remains owned by `runtime_environment.py`.
- Product credential adapters are downstream responsibilities and are not retained in the SDK.

## Boundary

This iteration covers Python only. Go has an equivalent `provider/easynet` lifecycle/identity seam and remains the next cross-language parity item.
