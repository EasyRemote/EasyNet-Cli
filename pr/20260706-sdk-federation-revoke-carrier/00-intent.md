# SDK Federation Revoke Carrier

## Intent

Expose the daemon-owned `federation.revoke` payload shape through the Go
Daemon SDK so product consumers do not import Axon SDK helpers for this carrier.

## Scope

- Add a small public Go SDK helper for `federation.revoke` args.
- Keep URA parsing/canonicalization out of the helper.
- Cover the helper with a focused SDK test.

