# API Contract

- Key-service request/response JSON frames are unchanged.
- Runtime signing identity behavior is unchanged.
- New product-neutral imports:
  - `easynet_sdk.providers.runtime.key_service`
  - `easynet_sdk.providers.runtime.keyring`
- Retired product-specific imports are not kept as aliases.
- Runtime keyring provider class is `RuntimeKeyringSignatureProvider`; the retired daemon-named class is not exported.
