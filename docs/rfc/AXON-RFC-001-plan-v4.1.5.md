# AXON-RFC-001 plan v4.1.5 — keyring vault + role overlay

Status: ratified by CTO 2026-05-03 (during Phase 3D + 3E live smoke)
Supersedes §3 (key persistence) of plan v4.1.4

This plan revision addresses a load-bearing collision that v4.1.4
introduced but did not solve: when one host runs both the backend
(hub identity, `easynet:///r/<realm>/hub`) and a daemon (device
identity, `easynet:///r/<realm>/device/<uuid>`), the two URAs share
the same realm but require **two different signing keypairs** —
one for hub-as-caller envelopes, one for device-as-caller. v4.1.4
loaded each from a different on-disk file
(`~/.easynet-hub/<realm>/identity.json` for hub, daemon-derived
seed for device). Operationally that meant:

- Two key files to back up, audit, rotate.
- No single source of truth for "what keys does this host hold?"
- The CLI's `easynet device join` had nowhere safe to drop a fresh
  device keypair — it persisted indirectly through the daemon's
  keypair derivation, not as an explicit secret.

v4.1.5 collapses these into a single **keyring vault** with a
**role overlay** model: one Ed25519 keypair, multiple URAs.

## §1 — The keyring vault

```
~/.easynet/keyring.enc   (or EASYNET_KEYRING_VAULT_PATH)
```

On-disk format: JSON envelope around AES-256-GCM ciphertext;
argon2id KDF (64 MiB / 3 iter / 1 parallelism / 32-byte key)
deriving the AEAD key from `EASYNET_KEYRING_PASSPHRASE`. Mode 0600.

### 1.1 Plaintext shape (after AEAD decrypt)

```rust
struct VaultPlaintext {
    entries: Vec<KeyringEntry>,
}

struct KeyringEntry {
    primary_self: String,      // canonical URA the key was minted for
    role_overlays: Vec<String>, // other URAs this same keypair signs as
    sealed_seed: [u8; 32],     // raw Ed25519 seed
}
```

### 1.2 Lookup

`Vault::lookup(self_uri)` matches `primary_self` first (O(1) via a
map cache), then scans `role_overlays[]` across every entry. The
first match wins. Hit rate is high because production hosts have
≤ 5 entries and each entry usually carries at most 2 overlays.

The match returns the `KeyringEntry` (NOT the seed). Every signing
operation goes through the vault's `sign_canonical(self_uri, bytes)
-> Signature` method, which performs the lookup and signs without
ever exposing the seed to the caller.

### 1.3 The one seed-exfiltration API

`Vault::export_seed(self_uri) -> Option<[u8; 32]>` is the **only**
method that returns raw seed bytes. It exists for one reason: the
daemon needs to construct a `SigningKey` once at boot to seed
existing in-process signing surfaces (the `EnvelopeSigner` pool,
the SDK's `AxonClient`-derived auth) which the v1 wire shape
requires. Future Phase 4 sweeps these consumers to use
`sign_canonical` directly so `export_seed` can be retired.

Calls to `export_seed` log a structured event for audit trail.

## §2 — Role overlay: one keypair, many URAs

A v4.1.5 host that runs both backend and daemon has one keyring
entry like:

```rust
KeyringEntry {
    primary_self: "easynet:///r/localhost/hub",
    role_overlays: vec![
        "easynet:///r/localhost/device/<host-uuid>",
    ],
    sealed_seed: [...],
}
```

The backend signs as `…/hub` (lookup hits primary). The daemon
signs as `…/device/<uuid>` (lookup hits overlay). Same seed, same
public key, byte-identical Ed25519 signatures — only the URA in
the envelope's `caller.uri` field differs.

Cross-language interop is proven live: Rust keyring daemon writes
the vault, Go reader at `backend/cmd/keyring-cross-test` opens it,
both resolve primary + overlay to the same keypair. Run the helper
to re-verify after any vault format change.

## §3 — Producer / consumer split

```
Producer:   EasyNet-Cli/src/bin/easynet-keyring.rs (Rust)
Consumers:  EasyNet/backend/internal/keyring/reader.go (Go)
            EasyNet-Cli/src/services/invocation_transport/boot.rs::
              try_load_daemon_seed_from_keyring (Rust)
```

The Rust keyring daemon is the **only** writer. The vault file is
written exclusively through `easynet device join` (which calls
into the daemon over UDS at `~/.easynet/keyring.sock`); every
other on-disk write is forbidden by the keyring daemon's API.

Both consumers READ the same file with the same KDF. Neither
consumer rewrites the vault; if a consumer needs to add an entry,
it talks to the keyring daemon over the UDS. This avoids the
"three writers race" pattern that broke v4.1.3's identity tree.

## §4 — Activation gate

The keyring path is **opt-in via env var**: `EASYNET_KEYRING_PASSPHRASE`.
When unset, both consumers fall through to the legacy path:

- Backend Go `LoadOrInitHubIdentity`: falls through to
  `~/.easynet-hub/<realm>/identity.json` (v4.1.4 shape).
- Daemon Rust `daemon_identity_from_stored`: falls through to
  the deterministic-derive path (seed = HMAC of node_id +
  process-local salt).

The opt-in gate matters because production hosts that haven't
migrated must keep working. A backend that boots without the env
var sees `ErrVaultMissing`, logs an info message, and proceeds on
the legacy path. A backend that boots with a vault that fails to
decrypt (wrong passphrase, corrupted file) logs a warning and
**still falls through to legacy** — a misconfigured vault must
never be a fatal boot error, since vault liveness should not gate
the user-facing HTTP listener.

## §5 — `easynet device join` flow under v4.1.5

```
operator: easynet device join <token>
  ↓
CLI:
  1. POST /api/v1/devices/pairing/<token>/preflight
  2. Generate Ed25519 keypair locally.
  3. Send {pubkey, hostname} to backend's
     /api/v1/devices/pairing/<token>/validate.
  4. Backend stamps device_pairings.device_public_key + returns
     credentials.json shape.
  5. CLI talks to keyring daemon over ~/.easynet/keyring.sock:
       insert(primary_self="easynet:///r/<realm>/device/<uuid>",
              role_overlays=[],
              seed=<the keypair's seed>)
  6. CLI writes credentials.json with no seed (the seed lives
     only in the vault now); credentials.json carries node_id,
     credential_token, hub_endpoint, realm.
```

Pre-v4.1.5 step 5 didn't exist — the CLI persisted the seed
either in credentials.json or in the daemon-derive path. v4.1.5's
single-source-of-truth invariant requires it land in the vault.

## §6 — `dev-backend.sh` integration

The dev script spawns the keyring daemon as a sandboxed sidecar:

```
$DEV_HOME/.easynet/keyring.sock      (UDS)
$DEV_HOME/.easynet/keyring.enc       (vault, mode 0600)
EASYNET_KEYRING_PASSPHRASE=<dev passphrase, generated per-reset>
```

`--reset-db` wipes `keyring.enc` because vault entries are URA-keyed
and a realm rollover (e.g. `localhost` → user-id realm for the
strict-flow demo) invalidates them. Same wipe semantics as the
realm-trust + identity-tree wipes in Phase 2G.

**Bug found + fixed during 3E live smoke:** the reset block was
gated by `DO_SEED=1`, so `--reset-db --no-seed` was a silent
no-op. Gate now reads `DO_SEED=1 || RESET_DB=1` — reset semantics
are independent of seed semantics.

## §7 — Migration: v4.1.4 → v4.1.5

| Phase | Scope                                                          | Commit       |
|-------|----------------------------------------------------------------|--------------|
| 3A    | `easynet-keyring` device identity vault binary                  | `e444faf`    |
| 3B    | `SelfIdentity` client abstraction (hides seed-vs-vault choice) | `00ca1d0`    |
| 3C    | `easynet device join` writes keyring entry, not credentials seed| `2fa5434`    |
| 3D    | Backend Go `LoadOrInitHubIdentity` consumes vault when opted in | (this rev)   |
| 3E    | Daemon Rust `daemon_identity_from_stored` consumes same vault   | (this rev)   |
| 3F    | Host e2e + this spec doc                                        | (this commit)|

Migration policy stays **wipe-and-rejoin** for dev environments;
production deploys land 3A–3E off (env var unset) by default and
flip the env var on after a controlled passphrase rollout.

## §8 — Parser strictness contract (carried forward from v4.1.4)

The 6-role URA grammar is unchanged:

```
easynet:///r/<realm>/<role>/<dot-to-thing>
role ∈ { user, device, agent, ability, hub, resource }
```

ParseURA strictness is unchanged from v4.1.4 §5. Role-overlay
matching is a vault-level concern, not a parser concern — the
parser sees a string and validates its shape regardless of
whether one or many keypairs sign for it.

## §9 — AXIOM seven-tuple correspondence (carried forward)

| slot     | URA kind constraint                                                |
|----------|-------------------------------------------------------------------|
| caller   | hub / device / agent (the entity that signed and transmitted)     |
| callee   | hub / device / agent (the entity addressed)                       |
| subject  | user / device / resource (principal acted upon or acting on)      |

Admission's `ValidateSubject` enforces the kind constraint. v4.1.5
does not extend the rule; the device-as-subject ratify from
v4.1.4 §3 still requires a Phase 4 follow-up to land in code.

## §10 — Open questions deferred (carried forward)

- **Multi-hub per realm (RFC-005)**: hub URI extends to
  `…/hub/<id>` when multi-hub topologies land. The keyring's
  primary_self / role_overlays scheme is forward-compatible; a
  multi-hub host would carry multiple `…/hub/<id>` overlays
  pointing at the same keypair (or distinct keypairs as policy
  dictates).
- **Vault rotation API**: changing `EASYNET_KEYRING_PASSPHRASE`
  today requires `easynet-keyring rotate <new-passphrase>`. A
  zero-downtime rotation that keeps both old and new keys decryptable
  during the swap is a v4.1.6 concern.
- **Per-entry rotation**: rotating a single Ed25519 keypair while
  preserving its URA bindings is not yet covered by the keyring
  API surface. Today the only "rotation" is "delete entry +
  re-pair" which loses the URA → key mapping in the trust anchor
  for the duration of the swap. Phase 4.

## §11 — Verification

| Surface                                          | Status |
|-------------------------------------------------|--------|
| Rust services tests                              | 412/412 PASS |
| Go `internal/keyring/` tests                     | 5/5 PASS |
| Go `TestLoadOrInitBackendIdentityPrefersKeyring`| PASS |
| Rust `daemon_identity_prefers_keyring_seed_over_deterministic_derive` | PASS |
| Rust `daemon_identity_falls_back_when_keyring_env_unset` | PASS |
| Cross-language interop (`backend/cmd/keyring-cross-test`) | live verified |
| Backend full sweep (22/22 packages)              | PASS |
| Phase 3F host-mode e2e (`scripts/dev-host-e2e.sh`) | live PASS — 2026-05-03 |
|   ↳ device primary + hub overlay share one keypair | proven via host-e2e-probe |
|   ↳ deterministic Ed25519 byte-identical signatures over canonical message | proven |
