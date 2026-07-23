# Invariants

## Semantic

- The active federation directory stream is
  `federation.subscribe_directory_v2`.
- The retired v1 name must not appear as an active descriptor, route, constant,
  or public SDK selector.
- Descriptor inventory must not advertise a stream that lacks a matching
  dispatcher branch.

## Safety

- Callers must not be routed into an ambiguous v1/v2 directory event shape.
- There is no alias fallback from v1 to v2; callers migrate to the typed v2
  stream explicitly.

## Boundedness

- Stream lifecycle and recovery stay owned by the v2 stream dispatcher.
- Removing the v1 descriptor creates no new runtime state and no compatibility
  queue.

## Recovery

- A v1 call now fails at discovery/route resolution as an absent capability.
- Existing v2 lag recovery and heartbeat behavior are unchanged.
