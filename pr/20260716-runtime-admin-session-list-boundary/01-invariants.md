## Invariants

1. Runtime-admin session list responses require a canonical `sessions` array.
2. Retired `items` payloads are not a public SDK fallback.
3. Every session row must be an object; malformed rows fail before projection.
4. Empty `sessions: []` is valid and represents a daemon-owned empty page.
5. Go and Python enforce the same response boundary and preserve valid public
   result shapes.
