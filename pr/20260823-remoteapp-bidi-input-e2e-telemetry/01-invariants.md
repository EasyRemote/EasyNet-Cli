# Invariants — RemoteApp Bidi Input E2E Telemetry

1. The selected app/window identity remains the Invocation `subject`; the Bidi
   attach args may carry `session_id` and `session_token`, but not a replacement
   resource subject.
2. The probe must use the public `easynet ability bidi remote_desktop.attach`
   path, not a Rust-only test helper or private plugin function.
3. Pointer and key probe frames must carry frontend-compatible
   `sent_at_ms` and monotonic `client_sequence` fields.
4. A view-only app/window session must reject pointer and key frames with
   `input_scope_unsupported`; it must not report `input_applied`.
5. The Bidi rejection payload must echo `client_sent_at_ms` and
   `client_sequence` so browser, daemon, and host evidence can be correlated.
6. This evidence remains a bounded E2E proof of fail-closed input safety; it is
   not proof that native app/window input injection is product-complete.
