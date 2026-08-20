# Architecture

## Boundary

Federation directory streaming is a daemon-owned stream capability. The
canonical active public stream is `federation.subscribe_directory_v2`, backed by
typed `DirectoryEvent` snapshots and deltas.

## Removed Legacy Surface

`federation.subscribe_directory` was still present as a descriptor-only active
ability and described legacy snapshots/deltas. There is no production handler
registered for that exact ability name.

## Clean Target

Only v2 is published:

```text
descriptor inventory
  -> federation.subscribe_directory_v2
  -> StreamDispatcher::subscribe_directory_v2
  -> typed DirectoryEvent stream
```

No descriptor, route, SDK facade, or product shim may expose the retired v1
name.
