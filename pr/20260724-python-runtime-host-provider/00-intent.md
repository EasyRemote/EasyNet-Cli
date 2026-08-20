# Intent

## Goal

Move Python runtime host lifecycle and invocation transport provider seams out of `easynet_sdk.providers.easynet` into `easynet_sdk.providers.runtime`, and remove the unused EasyNet credentials identity adapter.

## Non-goals

- Do not keep EasyNet import aliases.
- Do not migrate the remaining Go lifecycle provider in this iteration.

## Acceptance criteria

- Python lifecycle and transport provider modules live under `providers.runtime`.
- Generated/public tests import runtime provider modules.
- `easynet_sdk.providers.easynet.identity` is removed instead of moved into runtime provider.
- C ABI runtime host start payloads use runtime vocabulary at the SDK boundary.
- Product-neutrality and SPEC v2 gates reject retired Python lifecycle/transport/identity provider paths.
