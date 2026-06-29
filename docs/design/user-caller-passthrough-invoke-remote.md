# Design: User-Caller Pass-Through for `runtime.invoke_remote`

**Status:** implementing (2026-06-10)
**Author:** Silan Hu <silan.hu@u.nus.edu>
**Repos touched:** EasyNet (backend) · EasyNet-Cli (daemon) · EasyNet-Axon (SDK, read-only — no change needed)

## Problem

A browser-signed, device-hosted ability invoke (e.g. `remote_desktop.create_session`,
`screen.snapshot`) loses the real user caller identity twice on the way to the
device, so the inner ability handler sees a fabricated `_system` caller instead of
the user who actually clicked.

Evidence (ledger `inv_1026189adb8b4376`):
- `caller = easynet:///r/_system/agent/_system.local` — a fabricated URA (realm
  `_system`, agent `_system.local` are not Axon-canonical).
- `remote_desktop.create_session` is fail-closed and rejects with
  `consent_receipt_required` because the caller is neither a receipt-bearer nor
  the device owner.

### Where the identity is dropped

1. **`backend/internal/daemon_grpc/remote_routing.go:238`** —
   `remoteTransportCaller` returns `HubURA(realm)` when `PresignedCallerSignature
   != nil`. The real user URA + the browser signature are discarded; neither
   reaches `RemoteInvokeParams` (which has no presigned-signature field).
2. **`EasyNet-Cli .../dispatch_shim.rs:457`** — the self-target dispatch path
   hardcodes caller `easynet:///r/_system/agent/_system.local` and calls
   `LocalRuntime::invoke_async` (system/trust-domain), so the inner ability's
   `EnvelopeContext.caller` is `_system`.

The outer `runtime.invoke_remote` frame-0 envelope is *correctly* backend-signed
over the AXIOM-7 tuple `(caller=backend/hub, callee=device, subject=device,
ability="runtime.invoke_remote")` — that layer must stay as-is. The user identity
belongs to the **inner** invocation, which today has no wire carrier.

## Approach — inner signed-envelope pass-through via metadata

`InvokeRemoteUp::Request` already carries a `metadata: HashMap<String,String>`
for exactly this purpose ("authority material such as `x-easynet-delegation`
when the inner subject is a user represented by a hub/backend caller"). We add
one more metadata item carrying the **inner user envelope + browser signature**,
and have the receiving daemon dispatch the inner ability with
`LocalRuntime::invoke_externally_signed_async` (real cryptographic admission)
instead of `invoke_async` (system trust-domain).

This is **not** an outer wire-break: the outer `invoke_remote` envelope, its
backend signature, and the frame shape are unchanged. We add an additive,
optional metadata key. Old daemons ignore it (fall back to `_system`); new
daemons honor it.

### New metadata key

`x-easynet-origin-caller` — JSON value:

```json
{
  "caller_ura": "easynet:///r/<realm>/user/<uid>",
  "signature_b64": "<browser ed25519 signature over the INNER canonical bytes>",
  "signer_pubkey_b64": "<32-byte raw ed25519 verifying key>",
  "nonce_b64": "<16-byte inner invocation nonce>"
}
```

The signature is over the **inner** AXIOM-7 canonical bytes
`(caller=user, callee=device, subject=<inner subject>, ability=<inner ability>,
args=<inner args>, nonce)` — the same bytes the browser already produces for the
signed-submit envelope. We are forwarding that signed envelope verbatim, not
re-deriving it.

### Layer changes

| Layer | File | Change |
|---|---|---|
| backend params | `internal/axon/invoke_client.go` | add `OriginCaller *OriginCaller` to `RemoteInvokeParams` |
| backend routing | `internal/daemon_grpc/remote_routing.go` | when presigned present, keep the **backend/hub** transport caller (unchanged outer signing) BUT populate `OriginCaller` from `req.Envelope` (user URA + presigned signature + nonce) |
| backend frame0 | `internal/daemon_grpc/invoke_remote.go` | serialize `OriginCaller` into the `x-easynet-origin-caller` metadata item |
| daemon wire | `invoke_remote_initiator.rs` | (no field change — metadata already exists) document the new key |
| daemon dispatch | `daemon_invocation_service.rs::dispatch_self_targeted_invoke_remote` | parse `x-easynet-origin-caller`; when present, build the inner `InvocationEnvelope` with the user caller + verify via `invoke_externally_signed_async`; else fall back to the existing `_system` path |
| daemon shim | `dispatch_shim.rs` | add `dispatch_rpc_local_externally_signed(runtime, envelope, signature, payload)` that calls `invoke_externally_signed_async` |
| consent | `remote_desktop/session_consent.rs` | once the real user caller arrives, the existing `owner_self_consent` carve-out (user URA matches paired user) grants — no `_system` special-case needed |

### Security properties

- The inner signature is verified by the **device's** `LocalRuntime` KeyResolver
  against the user's registered pubkey (the row written by
  `register_device_pubkey` into `realm-trust.toml`). A forged
  `x-easynet-origin-caller` fails signature verify → falls closed.
- The outer backend signature still gates who may *call* `runtime.invoke_remote`
  at the device boundary; this change does not widen that.
- Cross-user / cross-realm callers without a valid inner signature continue to
  hit `consent_receipt_required` (the `owner_self_consent` carve-out only matches
  the paired user/device identity in the device's own realm).

### Fallback / compatibility

- A daemon built before this change ignores the new metadata key and uses the
  `_system` path → remote desktop still blocked there, but no regression.
- A backend built before this change never emits the key → new daemon falls back
  to `_system` → unchanged behavior.
- No proto change, no `.proto` regen, no Axon SDK change.

## Test plan

- **backend**: `remote_routing_test.go` — presigned invoke for a device-hosted
  ability populates `OriginCaller` and keeps the outer transport caller =
  backend/hub; non-presigned invoke leaves `OriginCaller` nil.
- **backend**: `invoke_remote_test.go` — frame0 round-trips the
  `x-easynet-origin-caller` metadata item.
- **daemon**: `dispatch_shim` test — `invoke_externally_signed` path carries the
  user caller into `EnvelopeContext.caller`.
- **daemon**: `session_consent` test (existing) — real user caller grants
  `owner_self_consent`; foreign callers stay fail-closed.
- **e2e**: rebuild daemon, open remote desktop, Share → session created, no
  `consent_receipt_required`; ledger shows `caller = easynet:///r/<realm>/user/<uid>`.
