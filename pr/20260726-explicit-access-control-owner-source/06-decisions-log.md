# Decisions Log

- 2026-07-26: Treat omitted `owner_source` as a legacy compatibility seam. The canonical runtime model requires explicit owner-resolution provenance for policy checks.
- 2026-07-26: Kept the public field names/types stable, but changed the semantic contract so missing `owner_source` fails before provider dispatch.
- 2026-07-26: Refined architecture gate R18 to avoid matching canonical `owner_ura -> owner_user_id` parsing as actor fallback, then added R98 to lock the explicit owner-source contract across Rust, Go, and Python.
