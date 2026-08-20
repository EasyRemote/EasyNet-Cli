# Invariants

1. Session authority metadata carries `issuer_ura`, `subject_ura`, `audience`, `scopes`, `issued_at_ms`, `expires_at_ms`, and signature.
2. Session authority metadata does not expose `backend_ura`, `user_ura`, `session_id`, or `audiences`.
3. The signer lookup uses `issuer_ura`.
4. The envelope caller must match `issuer_ura`.
5. The envelope subject must match `subject_ura` for user and session subjects.
6. The envelope callee must be admitted by `audience`.
7. Canonical payload construction remains in Rust daemon SDK core, not in language facades.
8. Go, Python, and Node agree on the shared authority fixture and schema.
9. No legacy input aliases are retained.
