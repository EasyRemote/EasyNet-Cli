# API Contract

- New product-neutral imports:
  - `easynet_sdk.providers.runtime.lifecycle`
  - `easynet_sdk.providers.runtime.transport`
- Runtime host start request JSON uses `runtime_instance_id`, `runtime_bin`, and `authority_endpoint`
  instead of product lifecycle field names.
- Direct and C ABI invocation transport connection behavior is unchanged.
- Retired EasyNet provider imports are not kept as aliases.
