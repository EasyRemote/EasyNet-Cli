# Daemon Key Service v1

## Purpose

The daemon key service is the sole authority for locally-held private keys.
SDKs and product repositories consume public projections and sign-only
capabilities; they do not open key stores, derive master keys, or materialize
private-key bytes.

## Domains

One daemon service owns two explicit domains. They share custody, transport,
auditing, and lifecycle rules; they do not share record semantics.

| Domain | Identity | Required lifecycle |
| --- | --- | --- |
| `runtime_identity` | Runtime owner URA with optional role overlays | ensure, public-key projection, sign |
| `managed_signing` | Key ID bound to a subject URA | create, list, public projection, sign, rotate, revoke, expiry, peer trust |

`runtime_identity` is the host/runtime trust anchor. `managed_signing` is the
rotatable policy key inventory. Treating a managed key as a role overlay, or a
runtime identity as a rotatable inventory key, is invalid.

## Authority boundary

The service exposes length-prefixed JSON over the daemon-local endpoint. Every
request is one of:

- `ensure`, `derive_pubkey`, `sign` for `runtime_identity`;
- `inventory.create`, `inventory.list`, `inventory.public_key`,
  `inventory.sign`, `inventory.rotate`, `inventory.revoke`,
  `inventory.set_expiry`, `inventory.bind_subject`, `inventory.peer_add`, or
  `inventory.peer_list` for `managed_signing`.

The request contains only identifiers, policy inputs, and canonical bytes. A
response can contain a public-key projection, immutable metadata, or a
signature. It never contains seed, private-key, master-key, vault ciphertext,
or a decryptable vault path.

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
- The daemon derives signer policy from `(subject_ura, key_id, public_key)`;
  callers cannot supply or override it.

## Migration invariants

1. There is one daemon service and one local custody boundary.
2. FFI local signing validates daemon-issued signer policy before requesting a
   signature.
3. Go and Python SDKs expose only generic runtime/key-service DTOs.
4. Product repositories depend only on SDK capabilities and daemon endpoints.
5. `keyring.json`, `KeyringHandle`, and facade-local vault code are deleted
   only after every inventory operation is provider-backed by this service.
6. No read fallback or dual-write path is permitted.

## Cutover evidence

Completion requires:

- inventory state-machine tests for create/sign/rotate/revoke/expiry/binding;
- FFI local-sign test using the daemon service;
- Go and Python provider conformance for public key and canonical-byte sign;
- import gates forbidding `keyring.json`, `KeyringHandle`, and private-key
  fields in product/SDK production code;
- Hub and EasyRemote end-to-end runtime checks.
