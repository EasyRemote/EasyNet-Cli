API Contract
============

Accepted request fields:

- `agent_ura`
- `presented_pubkey_b64` when a caller key must pin a user/device key.

Rejected request fields:

- `presented_pubkey_hex`

The public behavior remains compatible for canonical callers because all
production outbound constructors already use `presented_pubkey_b64`.
