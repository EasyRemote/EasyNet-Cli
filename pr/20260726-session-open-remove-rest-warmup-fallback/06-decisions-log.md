# Decisions Log

## 2026-07-26

- Treat REST credential warmup as a second lifecycle/admission authority, not as
  harmless resilience code.
- Prefer fail-closed signed gRPC session/prelude errors over best-effort REST
  repair before `session.open`.
- Keep explicit start/join HTTP credential verification out of this commit. The
  removed seam is only the implicit pre-dial session repair path.
- Add both architecture and SPEC v2 gates so the deleted warmup module and
  lifecycle vocabulary cannot return under another name.
