# Architecture

`src/daemon/invocation/admission/authority_metadata.rs` owns the daemon-side canonical shape of delegation/session authority metadata.

Layering:

- Core runtime/admission: deserialize and validate authority metadata shape.
- Admission facade: verifies trust/signature and maps valid authority facts into policy decisions.
- LocalRuntime adapter: receives already admitted session facts and never reparses legacy metadata.

The correct abstraction is a strict signed authority wire contract. Unknown fields are not extension points because they can carry contradictory subject, caller, callee, scope, or expiry semantics that are outside the canonical hash/signature model.
