# Decisions Log

## 2026-07-24

- Decision: centralize all-zero placeholder detection in `core::identity`.
- Reason: the placeholder is a runtime principal validity rule, not an FFI/auth/config/authority-local rule.
- Decision: update architecture gates to require daemon authority metadata to call the core identity guard rather than own the sentinel.
- Reason: daemon admission must still reject raw all-zero metadata, but duplicating the sentinel in daemon code preserves the legacy compatibility seam this refactor removes.
- Decision: make the SPEC v2 Rust production scan recognize `cfg(test)`, `cfg(all(test, ...))`, and `cfg(any(test, ...))` test sections.
- Reason: gate semantics must distinguish production code from readable negative tests without forcing tests into opaque string construction.
