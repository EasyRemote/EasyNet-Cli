# Invariants

1. Canonical signing material is obtained from the injected Runtime transport;
   Node must not compute canonical Invocation bytes.
2. `PreparedInvocation` is immutable signing material and is never submit-ready.
3. `SignedInvocation` is the only submit-ready pre-runtime object accepted by
   `RuntimeClient.submitSigned`.
4. Caller signature material must be preserved byte-for-byte in the signed
   envelope passed to the transport.
5. DescriptorRef in `signing_material` must match the prepared tuple
   `descriptor_ref`.
6. Public input uses latest canonical snake_case DTO fields only; no legacy
   aliases are accepted.
