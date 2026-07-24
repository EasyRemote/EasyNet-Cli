# API Contract

## Insert

`insert`, `insert_tracked`, `insert_negotiated`, and `insert_negotiated_with_trust` return an error when `ura` is not a canonical Device, User, or Agent URA.

## Lookup/remove

Lookup and removal remain idempotent string-key operations because callers may check stale or already-removed keys.

## Errors

Invalid insertion returns an explicit string error naming the presence key and canonical principal requirement.
