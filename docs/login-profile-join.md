# EasyNet Login, Profile, and Join

This document defines the user-facing login and device-join model for EasyNet.
It intentionally separates account authentication, device membership, local
runtime state, and invocation identity.

## Core model

EasyNet uses four separate lifecycle states:

```text
Account session
  The human user is authenticated to a Realm.

Device membership
  This machine has been admitted into that Realm.

Local runtime state
  The daemon/runtime is running and connected.

Invocation identity
  Runtime calls are signed and verified as a Principal/Device identity.
```

`login` changes account session state.

`join` changes device membership state.

`start` / `runtime start` changes local runtime state.

Invocation identity is verified by the runtime/SDK receipt and proof model, not
by the product login command.

## Main user flow

For a normal user:

```bash
easynet login silan@acme
easynet join
easynet status
```

Or, in one step:

```bash
easynet join silan@acme
```

The one-step command is workflow sugar for:

```bash
easynet login silan@acme
easynet join
```

Each stage remains independently recoverable. If login succeeds and device
enrollment fails, the account session/profile remains available and the user
can retry:

```bash
easynet join --profile silan@acme
```

## Login targets

The common form is:

```bash
easynet login <login-hint>@<realm>
```

Examples:

```bash
easynet login silan@official
easynet login silan@acme
easynet login silan@acme.com
easynet login silan@hub.acme.internal
```

The user part is a login hint. It is not a verified Principal identity until
the Realm's auth provider returns an authenticated subject.

If the account identifier itself contains `@`, use the full form:

```bash
easynet login --user silan.hu@company.com --realm acme
```

For SSO flows where the user is discovered by the browser or device-code login,
the Realm may be supplied without a user hint:

```bash
easynet login acme
easynet login --realm acme
```

## Hub override

`--hub` is an advanced endpoint override:

```bash
easynet login silan@acme --hub https://hub.acme.internal
```

It is useful for private deployments, CI, test environments, and air-gapped
Hubs. It does not make Hub identity equivalent to Realm identity. A Hub selected
with `--hub` must still represent the requested Realm.

## Realm resolution

Bare Realm aliases are safety-sensitive. Resolution order is:

1. Existing local profile alias.
2. Built-in reserved aliases such as `official`.
3. Full domain names through standard well-known discovery.
4. Enterprise-preconfigured Realm directory.
5. No untrusted public search for bare aliases.

For example, if `acme` has never been configured locally or through enterprise
configuration, the CLI must not silently search a public directory for a Realm
called `acme`.

## Trust prompt

The first trust prompt for a Realm should explain what is being trusted:

```text
The following Realm is requesting trust:

  Realm name: acme
  Realm ID: urn:easynet:realm:01J...
  Issuer: https://hub.acme.internal
  Discovery source: Local enterprise configuration
  Realm signing key: SHA256:...
  TLS verified for: hub.acme.internal

Trust this Realm on this device? [yes/no]
```

The fingerprint must identify whether it is a Realm signing key, trust anchor,
enrollment issuer, or TLS endpoint. Key rotation should be verified through a
continuity proof from the previous trusted Realm key.

## Profiles

A Profile is not a Hub endpoint.

A Profile is a local Realm + Account selection state:

```text
Profile
  -> Realm identity
  -> discovered Hub endpoint
  -> Account session
  -> Device membership
  -> Runtime Principal
```

Profiles must support multiple accounts in the same Realm:

```text
silan@acme
admin@acme
service-account@acme
```

Profile state stores non-secret projection data and a credential reference. It
must not duplicate access tokens.

Example projection:

```json
{
  "current_profile": "silan@acme",
  "profiles": {
    "silan@acme": {
      "profile_name": "silan@acme",
      "realm_alias": "acme",
      "realm_id": "urn:easynet:realm:01J...",
      "issuer": "https://hub.acme.internal",
      "login_hint": "silan",
      "subject": "usr_01J...",
      "credential_ref": "keychain://easynet/silan-acme",
      "trust_anchor": "SHA256:...",
      "account_session": "authenticated",
      "device_membership": "enrolled"
    }
  }
}
```

Current implementation may use an owner-only local auth session while platform
keychain integration is introduced. Profile JSON remains non-secret. Selecting
`--profile` does not by itself authorize enrollment: the active account session
must match the selected profile's issuer and verified account subject before
the CLI can request a device enrollment capability.

## Profile commands

```bash
easynet profile list
easynet profile use silan@acme
easynet profile show
easynet profile remove silan@acme
```

Scripts should prefer explicit profile selection:

```bash
easynet join --profile silan@acme
EASYNET_PROFILE=silan@acme easynet status
```

This avoids dependence on an interactive global current profile.

## Realm and Hub diagnostics

Realm and Hub commands are diagnostic/operator commands:

```bash
easynet realm resolve acme
easynet realm inspect acme
easynet hub inspect https://hub.acme.internal
```

The ordinary user switches profiles, not Hubs.

## Join behavior

After login:

```bash
easynet join
```

The CLI uses the current profile to:

```text
✓ Select Realm/profile
✓ Use the authenticated account session
✓ Request a device enrollment or pairing capability
✓ Create or reuse this Realm's device credential
✓ Verify the enrollment result
✓ Establish device membership
✓ Start or connect the local runtime when requested
```

Repeated `join` should be idempotent from the user point of view. A device that
is already joined should report the current membership instead of silently
creating duplicate identities.

## Device key isolation

Default device credentials should be isolated per Realm:

```text
Local device
  ├── Realm official credential/key
  ├── Realm acme credential/key
  └── Realm another-company credential/key
```

Cross-Realm device linking must require explicit delegation or linking proof.

## Logout, leave, and profile removal

These are distinct lifecycle actions:

```bash
easynet logout
```

Clears the current account session. It does not remove device membership.

```bash
easynet leave
```

Removes this device from the Realm and revokes/deletes device membership.

```bash
easynet profile remove silan@acme
```

Removes local profile projection. It does not imply remote revocation.

Status must show the layers separately:

```text
Profile: silan@acme
Realm: acme

Account session: Authenticated
Device membership: Enrolled
Runtime: Connected
Trust: Verified
```

## SDK boundary

`login` does not belong in the canonical runtime SDK.

The SDK should expose generic runtime verification concepts:

```text
TrustDomain
Principal
DeviceCredential
EnrollmentProof
InvocationReceipt
ReceiptVerifier
TrustFact
PrincipalBinding
InvocationIdentity
```

The SDK must not expose product login UX, official Hub account flows, browser
login lifecycle, commercial account directory semantics, or EasyNet-specific
profile management.
