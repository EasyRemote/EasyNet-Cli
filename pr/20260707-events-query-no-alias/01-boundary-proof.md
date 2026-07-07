# Boundary Proof

## Ownership

Events subscription inputs are SDK Events profile query DTOs. They belong to the
generic daemon SDK model and should not be represented through duplicate
per-stream alias names.

## Canonical Model

The canonical public query names are:

- `DirectoryEventQuery`
- `DeviceEventQuery`
- `SessionEventQuery`
- `InvocationEventQuery`

The shared `EventsSubscriptionRequest` remains an internal/base carrier shape;
public profile methods use the stream-specific query DTOs.

## No Compatibility Alias

The retired `EventsDirectorySubscriptionRequest`,
`EventsDeviceSubscriptionRequest`, `EventsSessionSubscriptionRequest`, and
`EventsInvocationSubscriptionRequest` names are removed from maintained SDK
surfaces and rejected by scaffold checks.

## Product Boundary

No backend, EasyRemote, or UI event shape is introduced. The query DTOs continue
to lower to daemon-owned event subscription carriers.
