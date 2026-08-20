# Voice Call Aggregate Provider v1

Status: production provider available; assembly remains deployment-conditional (2026-07-14)

## Ownership

Any live `voice.*` descriptor and receipt has one owner: the product realm Hub
projected to Axon's canonical Authority URA. Device authority does not publish
or execute a voice route.
`voice.subscribe` is Stream, `voice.transcribe` is Bidi, and the call signaling
verbs are RPC. Geometry and authorization action are separate descriptor facts:
inspection uses `read`, media carriers use `stream`, and every signaling
mutation, including `voice.report_metrics`, uses `invoke`. Admission and
admission explanation read the same bound descriptor action.

## Capability state

| Capability | Geometry | State | Production inventory |
| --- | --- | --- | --- |
| `voice.create_call`, `show_call`, `join_call`, `leave_call`, `end_call`, `watch_call`, `report_metrics`, `list_calls` | RPC | Seam or ProviderBacked | Published only when the qualified realm-shared provider is assembled |
| `voice.subscribe` | Stream | Unsupported | Not published; no Hub media provider assembly port exists |
| `voice.transcribe` | Bidi | Unsupported | Not published; no Hub media provider assembly port exists |

Capability state has exactly four values. `Unsupported` means no provider port,
`Seam` means a port without an assembled production provider, `ProviderBacked`
means registry assembly received a qualifying provider, and `CutoverReady`
additionally requires executable delivery evidence. Registration alone never
upgrades `ProviderBacked` to `CutoverReady`.

Static descriptor files preserve the public contract and geometry. They are
not evidence that a handler is operational; the live authority catalog is the
operational inventory.

## State machine

```text
create                         second participant
  |                                   |
  v                                   v
Ringing --------------------------> Active
  |                                   |
  |                fewer than two <---+
  +------------- end -----------------+
                  |
                  v
                Ended
```

`Ended` is terminal. Repeated `end` is an idempotent read of the same terminal
facts. `join`, `leave`, and metrics mutations after `Ended` are rejected without
changing the revision or event log. Participant leave and metrics updates do
not create a second call lifecycle.

Each participant has an explicit `Joined -> Left` lifecycle. Duplicate join,
duplicate leave, rejoin after leave, and metrics after leave are rejected. A
failed participant transition leaves the aggregate revision and event log
unchanged. A call becomes Active only when at least two participants are in
`Joined`; a first participant on a creator-less call remains Ringing.

## Repository contract

Operational call signaling registration requires an explicitly injected
`VoiceCallRepository`. The provider key is `(hub_authority_ura, call_id)` and
every mutation commits with compare-and-swap against `revision`. A production
provider must make insert-if-absent and compare-and-swap atomic across every
Hub replica serving the realm.

`HubRealmVoiceCallRepository` is the production adapter. It is constructible at
daemon boot only from the explicit `EASYNET_HUB_VOICE_SHARED_ROOT` deployment
setting. The root must be an absolute filesystem mounted by every Hub replica
for the realm and must provide cross-host POSIX advisory locks, atomic rename,
and durable file/directory sync. Daemon state directories are never consulted.
The adapter serializes each realm through the shared lock, validates the realm
identity on every read, and atomically replaces one durable realm snapshot.

When no provider is injected, Hub/Both daemon startup succeeds and omits all
call signaling handlers and descriptors from the live catalog. There is no
unavailable repository or local-file fallback. The in-memory
`TestVoiceCallRepository` is compiled only under `cfg(test)` and cannot produce
production qualification evidence. Supplying it to assembly leaves Voice in
`Seam` and registers no handlers.

## Persistence invariants

1. Acknowledged create is visible through a fresh provider instance.
2. Acknowledged mutation increments revision exactly once with checked
   arithmetic; recovered or proposed `u64::MAX` revisions fail closed.
3. A stale expected revision never overwrites a newer aggregate.
4. Compare-and-swap accepts only `replacement.revision == expected + 1`; equal,
   skipped, and overflowing revisions are rejected.
5. A CAS conflict retries from a fresh aggregate, so failed attempts have no
   durable events or revision side effects.
6. Missing records are the only implicit empty state; corrupt state fails closed.
7. Recovered open calls carry no terminal facts; recovered ended calls carry
   both `ended_at_ms` and `end_reason`.
8. Aggregate authority is the canonical Authority URA for the product Hub and
   never a Device URA.
9. Every loaded aggregate must exactly match the requested Hub/call key. List
   rows expose their repository key separately, and the embedded aggregate must
   match both parts before it is returned.
10. The proposed aggregate key is unchanged before CAS, and a successful CAS is
    reloaded and compared with the complete proposed aggregate before success is
    returned to the caller.
11. Every mutation event carries the admitted invocation command id. CAS
    returns `Committed`, `Current`, or `Ambiguous`; an ambiguous acknowledgement
    succeeds only after a reload finds the exact proposed command event.
12. Recovery validates participant cardinality/lifecycle, aggregate and list
    keys, event sequence, command uniqueness, revision relation, terminal event
    placement, and all creation/join/leave/end time ordering before exposure.

## Deployment boundary

The daemon constructs the qualified provider only for Hub/Both mode when
`EASYNET_HUB_VOICE_SHARED_ROOT` is configured. Registry assembly validates the
provider qualification before moving signaling from `Seam` to
`ProviderBacked`. The consistency boundary is the shared realm mount, not one
daemon process, host, or state directory. Without the setting, production
starts normally without Voice routes and does not advertise unavailable
handlers. `CutoverReady` remains a separate deployment decision requiring
executable delivery evidence.
