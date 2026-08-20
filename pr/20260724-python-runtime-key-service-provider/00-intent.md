# Intent

## Goal

Move Python runtime signing custody helpers out of `easynet_sdk.providers.easynet` into `easynet_sdk.providers.runtime`.

## Non-goals

- Do not change the key-service wire protocol.
- Do not keep EasyNet import aliases.
- Do not migrate daemon lifecycle/transport in this iteration.

## Acceptance criteria

- Python managed signing imports key-service helpers from `providers.runtime`.
- Runtime identity/keyring helpers live under `providers.runtime`.
- Product-neutrality gates reject retired `providers/easynet/keyring.py` and `providers/easynet/key_service.py`.
- Managed signing and runtime identity tests pass.
