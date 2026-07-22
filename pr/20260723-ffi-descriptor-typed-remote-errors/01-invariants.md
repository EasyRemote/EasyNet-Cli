# Invariants

## Semantic invariants

- Descriptor resolution states are typed states, not interpretations of daemon
  message text.
- Caller signer unavailability is detected before daemon IO.
- Remote owner unavailability is reported only by the remote invocation route
  boundary, not by arbitrary substring matching.

## Safety invariants

- A malformed request cannot be reclassified as routing failure.
- A missing caller signer cannot be hidden behind descriptor-not-found.
- A remote route failure cannot force local descriptor fallback.

## Boundedness invariants

- The remote catalog probe performs one signed remote `meta.list_abilities`
  invocation.
- There is no retry loop and no secondary route authority.
