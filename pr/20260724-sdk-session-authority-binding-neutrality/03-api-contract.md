# API Contract

Changed SDK facade receipt JSON for `authority_binding.kind == "session"`:

```json
{
  "kind": "session",
  "issuer_ura": "...",
  "subject_ura": "...",
  "session_id": "...",
  "scopes": ["..."],
  "audiences": ["..."],
  "issued_at_ms": 1,
  "expires_at_ms": 2,
  "signature_base64": "..."
}
```

Retired facade fields `backend_ura` and `user_ura` are not accepted by SDK
validators.

No public classes/functions are added or removed. DirectRuntime provider
projections adapt generated Axon schema fields into the generic SDK facade.
