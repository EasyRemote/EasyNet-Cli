# Intent: Admin trust/pairing C ABI runtime execution

Close the Admin + Gateway C ABI gap for hub lifecycle and pairing trust
operations needed by EasyRemote Server/Gateway cutover.

The implementation must expose daemon-owned carrier builders and projection
helpers for:

- `hub.join`
- `hub.leave`
- `pairing.preflight`
- `pairing.create`
- `pairing.validate`
- `credential.verify`

Go and Python C ABI transports must execute these operations through Runtime
Core invoke, then project daemon output into SDK DTOs. They must not synthesize
trust semantics or parse Axon protocol truth in the language facade.
