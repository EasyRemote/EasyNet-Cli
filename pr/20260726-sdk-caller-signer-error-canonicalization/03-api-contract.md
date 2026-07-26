API contract
============

Input
-----

A runtime error DTO containing:

- `code = CALLER_SIGNER_UNAVAILABLE`
- any valid `stage`
- any valid `retry`
- a message that may contain raw custody implementation detail

Output
------

The decoded SDK error keeps:

- `code`
- `stage`
- `retry`
- `retryable`
- `source`
- `invocation_id`
- `receipt_ura`
- `details`

The decoded SDK error message becomes:

```text
CALLER_SIGNER_UNAVAILABLE: remote invocation requires a caller signer for `<caller_ura>`; load or provision that identity in the local key service
```

when a caller URA is present, or the same text without the `for ...` clause when
no caller URA can be extracted.

Rejected projection
-------------------

The public SDK error message must not include:

- `keyring entry not found`
- `keyring rejected request`
- `self-identity:`
- `KeyService signer`
