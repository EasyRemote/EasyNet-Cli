# Decisions Log

## 2026-07-26

- Treat PTY vocabulary as implementation-level only.
- Keep public terminal ability strings unchanged.
- Use SPEC v2 to prevent reintroducing PTY-session ability constants.
- Allow `PtyService`, `PtySessionId`, and backend PTY driver names to remain because they describe the OS execution mechanism below the runtime ability boundary.
- Add the terminal lifecycle args guard to the main SPEC v2 path; a guard that only runs in self-test does not protect the production checkout.
