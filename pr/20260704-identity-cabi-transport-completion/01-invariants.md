# Invariants

- URA and DescriptorRef helpers remain delegated to the existing identity C ABI functions.
- ResourceRef projection delegates to the existing daemon-owned C ABI projector; Python does not build resource URAs.
- Signing-key lifecycle and signer construction must not fake keyring behavior while lower C ABI carriers are absent.
- Default C ABI identity clients must fail with typed SDKError, not AttributeError or facade-local placeholders.
