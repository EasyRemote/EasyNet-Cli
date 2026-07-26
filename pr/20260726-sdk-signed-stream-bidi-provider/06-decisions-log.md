# Decisions Log

2026-07-26:
- Chose signed RuntimeClient carrier APIs instead of downgrading AuthorizedRuntimeSession to unsigned drafts.
- Treat the existing provider-unavailable stream/bidi methods as obsolete stubs because transports already provide the carrier capability.
- Rebuilt SDK conformance inventory because the RuntimeClient public API and provider implementation hashes changed intentionally.
