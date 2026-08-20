# Intent

## Goal

Remove the FFI descriptor resolver's remote descriptor-probe fallback so public
descriptor resolution is a bounded catalog lookup over the local runtime owner
and its provider-backed realm catalog only.

## Non-goals

- Do not add a second route resolver.
- Do not synthesize descriptors for remote devices from static system catalog
  shape.
- Do not preserve remote probing as a compatibility path.
- Do not change the public SDK request shape; callers may continue to send
  `caller_ura` and `subject_ura`, but descriptor resolution must not use them
  to perform hidden remote invocation.

## Acceptance Criteria

- `runtime_resolve_descriptor_ref_json` never invokes a remote ability while
  resolving a descriptor ref.
- A remote descriptor miss returns a typed `DESCRIPTOR_NOT_FOUND` from the
  bounded catalog authority, not `CALLER_SIGNER_UNAVAILABLE`, owner-offline, or
  timeout from a fallback probe.
- Obsolete `RemoteDescriptorCatalogProbe` and probe-subject state are removed.
- Architecture gates reject reintroduction of remote descriptor-probe fallback.
- Targeted Rust tests prove local catalog success and remote miss fail-closed
  without daemon IO.
