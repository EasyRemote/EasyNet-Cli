# Hot Agent Authority Aggregate Snapshot API Contract

## Public API

No public API change.

## Error Contract

Existing `HotAgentAuthorityInventoryError` variants remain the authority-domain surface:

- `DurableRegistryUnreadable`
- `DurableAgentMissing`
- `IdentityRegistryUnreadable`
- `IdentityMissing`
- `IdentityAmbiguous`
- `IdentityInvalid`
- `DurableAgentStillPresent`
- `IdentityStillPresent`

## Authority Contract

Persisted hosted-Agent identity remains stronger than declared/static authority roots. Static roots cannot enroll or revoke a hot Agent unless the durable aggregate state proves the same lifecycle fact.
