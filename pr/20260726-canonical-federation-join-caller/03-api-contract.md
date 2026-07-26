Public contract
===============

- `federation.join` request JSON still uses `membership_ura`, `realm`, and
  `public_key_hex`.
- The wire envelope caller and subject are the membership Device URA.
- The callee is the realm Authority URA.

Error contract
==============

- Non-join use of bootstrap ingress is denied.
- Caller/subject/membership mismatch is denied before key lease.
- Malformed `public_key_hex` is rejected as invalid argument.
- Unknown arbitrary callers are not repaired by bootstrap key leasing.

Tenant rules
============

- Caller, callee, subject, and membership realms must match.
- Membership URA must identify a Device.
- Callee URA must identify an Authority.
