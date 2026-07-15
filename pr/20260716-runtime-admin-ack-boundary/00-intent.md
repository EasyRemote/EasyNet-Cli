## Intent

Close the runtime-admin revoke-result ownership fork where SDK facades could
project a missing or malformed daemon `ack` field as successful revocation.

Expected effect: architecture convergence. The daemon/runtime-admin ability is
the source of truth for administrative success; Go and Python SDKs validate that
canonical `ack` is present and boolean before exposing a revoke result.
