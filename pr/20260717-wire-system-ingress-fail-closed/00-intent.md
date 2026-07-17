# Wire System Ingress Fail-Closed

## Root Fork

RF-8 remains open while `wire_descriptor` can replace missing wire caller,
subject, or nonce fields for a local-system dispatch. That behavior creates a
second tuple-derivation authority beside `SystemInvocationIssuer`.

## Change

- Require every wire envelope reassembled by the daemon to contain the complete
  caller, callee, subject, nonce, causal context, descriptor ref, and payload.
- Remove the `WireCallerIdentity::LocalSystem` fallback state.
- Make local-system request signing reachable only through
  `SystemInvocationIssuer`.
- Seal trusted-local dispatch classification inside the daemon crate so an
  external Rust caller cannot construct or request system authority.
- Preserve the existing local loopback transport contract because its typed
  builder already emits the complete tuple.
- Add an architecture gate and failure-path tests that reject reintroduced
  caller, subject, or nonce synthesis at wire ingress.

## Ownership

Axon owns descriptor-bound canonical bytes and admission. EasyNet-Cli owns the
trusted-local transport classification. `SystemInvocationIssuer` is the only
daemon authority allowed to convert that classification into a signed
`_system.local` runtime request.
