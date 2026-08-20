# Architecture

## Root abstraction problem

`managed_signing.py` imports the daemon key-service transport from `providers.easynet`, which makes a canonical SDK signing API depend on a product-named provider namespace.

## Target model

- `providers.runtime.key_service` owns the local runtime custody transport protocol.
- `providers.runtime.keyring` owns runtime signing identity projection and signing delegation.
- Managed signing depends on runtime provider custody helpers only.
- `providers.easynet` remains limited to still-unmigrated lifecycle/transport seams.

## Boundary

This iteration migrates Python key-service/keyring custody helpers only. Lifecycle and transport remain separate seams because they own daemon process/session behavior and require separate cross-language parity work.
