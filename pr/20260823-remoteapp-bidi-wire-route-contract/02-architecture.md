# Architecture

Invariant:

- `call_mode` remains the governed interaction mode: rpc, stream, or bidi.
- `bidi_wire_kind` is descriptor metadata for bidi data-plane representation.
- `bidi_wire_kind` is not an eighth Invocation tuple field.
- Plugin package manifest is the source of truth for plugin wire profile.
- Generated plugin ability TOML, Runtime descriptor metadata, backend route DTO,
  and frontend route model must carry the same value.
- Non-bidi descriptors with `bidi_wire_kind` are rejected when projected into a
  governed descriptor.

Layer order:

1. EasyNet-Cli plugin manifest/package metadata.
2. EasyNet-Cli governed `AbilityDescriptor.metadata`.
3. EasyNet backend catalog projection.
4. EasyNet frontend route normalization and RemoteDesktop readiness helpers.

