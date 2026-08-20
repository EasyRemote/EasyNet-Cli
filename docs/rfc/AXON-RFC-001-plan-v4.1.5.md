# AXON-RFC-001 — canonical daemon key service

Status: implemented architecture baseline (the historical role-overlay design
formerly kept at this path is retired).

## Decision

EasyNet-Cli owns one local key-service process. Product runtimes and language
SDKs use a sign/public-projection protocol and never open the encrypted vault,
import seeds, derive production keys, or persist private material.

Each canonical runtime owner URA has a distinct Ed25519 key. Co-location is not
authority: a Device key cannot sign as a Hub, a Hub key cannot sign as a
Device, and an Agent or User key cannot be represented as an alias of another
role. The previous `role_overlays` model is deleted.

## Runtime identity model

The runtime-identity service has six bounded operations:

1. `health()` — return only the protocol version; never enumerate keys.
2. `ensure(primary_self)` — atomically generate a key inside service custody
   when the exact owner URA is absent; return only its public key.
3. `derive_pubkey(self_ura)` — return the exact owner's public projection.
4. `runtime.list(limit, cursor)` — return one ordered owner page.
5. `sign(owner, public_projection, policy_ref, canonical_bytes)` — validate a
   projection-bound intent and sign as that exact owner.
6. `forget(primary_self)` — delete the exact owner's key.

There is no key alias operation and no seed import/export operation.

Daemon modes provision authority independently:

| Mode | Device identity | Hub identity | Device credentials required |
|---|---:|---:|---:|
| Device | yes | no | yes |
| Hub | no | yes | no |
| Both | yes | yes, distinct key | yes |

Device mode retains the Hub public key learned during pairing in the realm
trust anchor. It never replaces that row with a local Device key.

## Managed signing model

Managed signing is a generic, rotatable key domain separate from runtime
identity. Its explicit lifecycle is:

```text
active -> retired -> revoked
active -----------> revoked
```

Only active, unexpired keys may sign. Subject binding is immutable for a key;
rebinding requires rotation. Public projections carry key ID, public key,
lifecycle state, rotation epoch, subject, signer-policy reference, timestamps,
and predecessor ID. They never contain storage or private-key fields.

Inventory and peer reads are cursor-paginated. A response contains at most 16
records. The SDKs may provide compatibility helpers that walk pages, but every
wire exchange remains bounded and detects repeated cursors. Compatibility
walkers stop at 1,024 pages or 16,384 records. Filtered key pages scan at most
256 ordered records and may return an empty page with an advancing cursor.

Managed signing requests carry `(key_id, subject_ura, signer_policy_ref)`.
The custody service checks that tuple against the immutable public projection
before signing; an opaque key ID plus arbitrary bytes is not a public API.

## Transport and custody bounds

- UDS / named-pipe request framing is `u32-be length || JSON`.
- Canonical signing input is bounded at 64 MiB, aligned with Invocation gRPC.
- The base64 JSON frame is bounded at 90 MiB in Rust, Go, and Python.
- The socket is owner-only (`0600` on Unix).
- Four connections are admitted concurrently; a connection handles at most
  256 requests. One 30-second absolute deadline covers each complete frame.
- The encrypted vault is owned under a process-spanning exclusive file lease;
  a second service owner fails startup instead of opening an independent
  snapshot.
- Master-key failure, corrupt storage, malformed protocol data, and unavailable
  service states fail closed.
- A single lifecycle manager owns attach/spawn/restart and every Child handle.
  A failed readiness transition kills and reaps the owned child.
- Generated passphrase storage is single-assignment `create_new`, mode `0600`
  at open, syncs both file and parent directory before success, and never
  rewrites malformed existing state.

## Consumer boundary

Production consumers receive narrow owner/key-bound signer capabilities.
Administrative inventory, rotation, revocation, expiry, and peer-trust
operations are not mixed into the signer interface. Go and Python implement
the same request/response, lifecycle, pagination, deadline, and error model.

EasyNet and EasyRemote are consumers of these generic runtime capabilities.
Neither product contributes product-specific naming, directory layout,
receipts, authentication, or lifecycle concepts to the SDK.

## Forbidden regressions

Static and test gates must reject:

- public seed/private-key/vault/passphrase request or response fields;
- `export_seed`, seed import, deterministic production signing, or file-vault
  fallback in SDK or product code;
- runtime role aliases or Device-to-Hub key sharing;
- unbounded inventory/peer reads;
- two processes opening the same encrypted vault concurrently;
- Hub-only boot depending on Device credentials;
- Device mode publishing its local key as the realm Hub key.
