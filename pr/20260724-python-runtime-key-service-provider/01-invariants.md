# Invariants

1. Signing custody is a canonical runtime provider capability, not an EasyNet product capability.
2. No Python canonical SDK module may depend on `easynet_sdk.providers.easynet`.
3. No compatibility import package remains for retired key-service/keyring paths.
4. Public behavior and object semantics remain stable except for product-specific package ownership.
