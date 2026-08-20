# API Contract

## Row identity

- `agent_ura` identifies the row to replace.

## Device row

- Required: `agent_ura`, `public_key_b64`, `role = "device"`, `added_at_unix_ms`.
- Forbidden after normalization: `origin_realm`, `hub_endpoint`, `tls_ca_pem_path`.

## Hub row

- Required: `agent_ura`, `public_key_b64`, `role = "hub"`, `added_at_unix_ms`, `origin_realm`, `hub_endpoint`.
- Optional: `tls_ca_pem_path`.

## Errors

- Invalid TOML or non-array `trusted_agent` remains a hard error.
- No stale-row compatibility success path is retained.
