## Invariants

1. Runtime-admin revoke success is never fabricated by the SDK.
2. `ack` is a required daemon-owned boolean in revoke responses.
3. Optional readiness flags may be absent and default to false, but if present
   they must be booleans.
4. Go and Python SDK facades enforce the same response boundary.
5. The public revoke request and result shapes remain unchanged for valid
   daemon responses.
