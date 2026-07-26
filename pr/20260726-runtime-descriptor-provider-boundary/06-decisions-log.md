2026-07-26:
- Selected Workstream B because codegraph showed FFI invocation owns descriptor catalog construction and row resolution.
- Keep ABI stable and move authority downward instead of adding an adapter or compatibility fallback.
- Kept `runtime_owner_ura_from_session` in FFI because it is handle/control-discovery translation; passed it lazily into the provider so request validation and owner mismatch still fail before session-owner lookup.
- Updated both SPEC v2 and legacy architecture gates to validate the new provider boundary rather than preserving the old FFI-local authority.
