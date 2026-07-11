# Daemon Key Service v2

## Purpose

The daemon key service is the sole authority for locally-held private keys.
SDKs and product repositories consume public projections and sign-only
capabilities; they do not open key stores, derive master keys, or materialize
private-key bytes.

Every SDK request binds an explicit daemon endpoint. Endpoint publication,
local-directory layout, and environment discovery are product daemon lifecycle
policy; they are not part of the generic runtime SDK contract.

## Domains

One daemon service owns two explicit domains. They share custody, transport,
auditing, and lifecycle rules; they do not share record semantics.

| Domain | Identity | Required lifecycle |
| --- | --- | --- |
| `runtime_identity` | Exactly one runtime owner URA per key | ensure, public-key projection, sign |
| `managed_signing` | Key ID bound to a subject URA | create, list, public projection, sign, rotate, revoke, expiry, peer trust |

`runtime_identity` is the host/runtime trust anchor. `managed_signing` is the
rotatable policy key inventory. Runtime-role aliases are invalid: Device and
Hub owners on the same host still have distinct keys.

## Authority boundary

The service exposes length-prefixed JSON over the daemon-local endpoint. Every
request is one of:

- `health`, `ensure`, `derive_pubkey`, `runtime.list`, `sign` for
  `runtime_identity`;
- `inventory.create`, `inventory.list`, `inventory.public_key`,
  `inventory.sign`, `inventory.rotate`, `inventory.revoke`,
  `inventory.set_expiry`, `inventory.bind_subject`, `inventory.peer_add`, or
  `inventory.peer_list` for `managed_signing`.

The request contains only identifiers, policy inputs, and canonical bytes. A
response can contain a public-key projection, immutable metadata, or a
signature. It never contains seed, private-key, master-key, vault ciphertext,
or a decryptable vault path.

`health` is a constant-size protocol-version response (`protocol_version: 2`)
and never enumerates keys. Version 1 requests are not accepted. Request and
public-projection DTOs reject unknown fields. Both signing operations are typed
intents: runtime signing binds the
owner URA, cached public projection, and daemon-derived policy reference;
managed signing binds the key ID, expected purpose, immutable subject URA, and
daemon-derived policy reference. The service rejects any mismatch before
private-key use.
The raw wire enums and raw managed-sign method are crate-private; runtime code
receives owner/key-bound signer capabilities.

Signing accepts at most 64 MiB of canonical bytes and the framed JSON protocol
accepts at most 90 MiB after base64 expansion. Inventory and peer reads use
ordered cursor pages with a maximum of 16 records per response. Compatibility
collectors stop after 1,024 pages or 16,384 items. A filtered managed-key page
examines at most 256 ordered records and may therefore return an empty page
with an advancing cursor.

The service accepts at most four concurrent local connections. Each connection
handles at most 256 requests, and every request/response frame shares one
30-second absolute deadline from length read through response flush. A timeout
is terminal for that connection.

## Managed signing state machine

```text
create -> active -> retired -> revoked
                 \-> revoked
```

- Only `active` keys can sign.
- Rotation atomically retires the predecessor and creates one successor with
  `rotated_from` and a strictly greater epoch.
- Revocation is terminal.
- A bound subject is immutable for the lifetime of a key. Rebinding requires a
  successor key.
- The daemon derives signer policy from
  `(purpose, subject_ura, key_id, public_key)` under the stable domain
  `canonical-runtime.managed-signing.policy` and emits
  `managed-signing:v2:sha256:<digest-prefix>`; callers cannot supply or
  override it. There is no legacy hash fallback.
- A repeated peer registration may refresh `last_seen` and `via_hub` only when
  its public key is identical. Replacing a peer trust anchor requires a future
  explicit retrust/rotation state machine and is rejected by v2.

## Persistence commit model

The encrypted vault format is version 2. The outer envelope, decrypted
plaintext, and managed inventory reject unknown or missing fields. Version 1
files are rejected; there is no dual reader or implicit migration.

All vault replacement uses the shared atomic writer and one of two typed error
states:

- `NotCommitted`: rename did not occur; the in-memory mutation is rolled back.
- `ReplacementVisibleButDurabilityUncertain`: rename occurred but parent
  directory fsync failed; the new in-memory state is retained and the service
  fail-stops. Health and every subsequent request are rejected so the lifecycle
  supervisor restarts the process.

Opening an existing vault re-fsyncs its parent directory before accepting the
visible state. This is the only recovery from durability uncertainty.

## Migration invariants

1. There is one daemon service and one local custody boundary.
2. FFI local signing validates daemon-issued signer policy before requesting a
   signature.
3. Go and Python SDKs expose only generic runtime/key-service DTOs.
4. Product repositories depend only on SDK capabilities and daemon endpoints.
5. `keyring.json`, `KeyringHandle`, and facade-local vault code are deleted
   only after every inventory operation is provider-backed by this service.
6. No read fallback or dual-write path is permitted.
7. The service holds a process-spanning exclusive lease for the encrypted
   vault lifetime; concurrent independent snapshots are rejected.
8. Device mode never provisions or publishes Hub signing authority. Hub-only
   mode never reads Device credentials.
9. One process-wide lifecycle manager owns attach/spawn/restart transitions and
   every spawned child until it is reaped. Startup readiness failure always
   kills and waits for the owned child; a reachable but protocol-unhealthy
   owned child is also reaped before supervised restart. An unhealthy external
   service is never preempted.
10. The key-service process is the only reader/creator of the single
    passphrase file. Lifecycle code only attaches or spawns and never reads or
    forwards the secret. The file is single-assignment: `create_new`, exactly
    64 lowercase hexadecimal bytes, Unix mode `0600`, `write_all`, file
    `fsync`, and parent-directory `fsync`. Existing files are opened without
    following symlinks, validated as regular files, and re-fsync their parent
    before use. Empty, malformed, insecure, or unreadable state fails closed
    and is never overwritten.

## Local trust boundary

The endpoint's `0600` ACL defines one local OS-user trust principal. Exact
Device/Hub/subject keys prevent authority aliasing and typed signer objects
prevent accidental cross-owner selection inside the runtime. The service does
not claim to isolate mutually hostile processes running as the same OS user;
that would require a separate OS process-attestation authority and would form
a second authentication system outside this specification.

## Cutover evidence

Completion requires:

- inventory state-machine tests for create/sign/rotate/revoke/expiry/binding;
- FFI local-sign test using the daemon service;
- Go and Python provider conformance for public key and canonical-byte sign;
- import gates forbidding `keyring.json`, `KeyringHandle`, and private-key
  fields in product/SDK production code;
- Hub and EasyRemote end-to-end runtime checks.
