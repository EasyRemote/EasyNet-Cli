# Invariants

- The public API inventory remains complete for both Go and Python SDK exports.
- Runtime environment projection symbols are product-neutral runtime concepts:
  state root, credentials projection path and paired runtime identity.
- `SdkEnvironment` methods are inventoried as public members instead of being
  invisible convenience methods.
- No product account, HTTP session, EasyRemote workflow or private-key custody
  concept is introduced into the SDK inventory.
