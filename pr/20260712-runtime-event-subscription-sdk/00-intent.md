Runtime event subscription SDK cutover
======================================

Intent
------

Move daemon runtime event subscription draft construction from EasyNet Backend's
`sdkevents` adapter into the canonical EasyNet-Cli Go/Python SDK runtime-events
facade.

Boundary
--------

- SDK owns generic event stream kind, cursor and daemon ability lowering.
- Backend owns product request DTOs and HTTP/SSE projection.
- Backend must not maintain a second stream-kind-to-daemon-ability table.

Invariants
----------

1. Public Backend `sdkevents.Port` behavior and draft output remain compatible.
2. Runtime event subscription draft construction preserves the full
   RuntimeCallContext tuple.
3. Go and Python SDKs expose symmetric event subscription request, cursor,
   stream kind and builder behavior.
4. Session event attachment remains a stream-specific lowering rule in the SDK,
   not in Backend.
5. Tests prove descriptor refs, args, metadata and unsupported stream rejection.
6. Resume cursors are stream-scoped durable continuation facts: a cursor from
   one runtime event stream must not resume another stream, and caller-provided
   opaque tokens must be canonical instead of trim-repaired.

Delta — 2026-07-12
------------------

The runtime events SDK now treats subscription resume cursors as typed,
stream-scoped continuation state. Go and Python providers reject cross-stream
resume cursors before lowering a daemon InvocationDraft. Python also rejects
negative resume sequences, matching Go's unsigned sequence model.
