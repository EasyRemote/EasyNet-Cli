# RemoteApp Shared Media Lane v2

Status: Unix production path and real Linux/X11 process proof implemented;
Windows named-mapping production dispatch implemented and cross-compiled;
real Windows and cross-device product evidence incomplete.

## Scope

This is a plugin-private transport between one supervised
`easynet-remoteapp-media-host` generation and the daemon-owned WebRTC carrier.
It does not change public Invocation, Ability, receipt, authority, consent,
session, SDP, ICE, or C ABI semantics.

The existing fixed `RVID` and `RAUD` headers remain canonical. Shared memory
changes where their bytes live; it does not create another media lifecycle.

## Required outcome

- The host copies each encoded H.264 or Opus payload once into a preallocated
  shared slot.
- The notification pipe carries only a fixed slot ticket, never codec bytes.
- The daemon validates the fixed header and codec payload in place, then makes
  exactly one bounded detach copy into transport-owned `Bytes` before handing
  the payload to WebRTC.
- Video and audio retain separate memory, notification, backpressure and
  failure domains.
- Memory is fixed at generation creation and cannot grow with sender or
  receiver delay.

This is not an end-to-end zero-copy claim. Platform encoders may produce owned
output, the host performs one copy into the shared slot, the daemon performs
one ownership-detach copy, and RTP/SRTP/network stacks may packetize or copy
later. The detach is required because packetization and NACK retransmission may
retain payload bytes longer than a shared-ring producer can safely wait.

## Generation-owned layout

Each lane owns one generation-scoped mapping. Unix uses an anonymous, unlinked
file inherited across `exec`; Windows uses a `Local\\` named file mapping whose
name is scoped by process id, random generation nonce and physical lane:

```text
control page (read/write in both processes)
  magic, version, lane, slot_count, slot_capacity, generation nonce
  slot[0..N] { atomic state, atomic ticket, frame_len }

payload mapping (read/write host, read-only daemon)
  slot[0] fixed RVID/RAUD header + codec payload
  ...
```

Slot count is fixed to the negotiated queue contract: at most three video
slots and four audio slots. Video slot capacity is the negotiated
`max_access_unit_bytes` plus its fixed header; audio capacity is the maximum
validated Opus packet plus its fixed header. Lengths are rejected before any
slot mutation.

## Slot state machine

```text
FREE --producer CAS--> WRITING --release--> READY
READY --consumer CAS--> READING --validate + detach/drop lease--> FREE
READY --producer CAS/drop-oldest--> WRITING
```

`READING` is immutable and cannot be reclaimed by the producer. Every publish
increments a non-zero ticket. Notifications contain `{slot_index, ticket}`;
the consumer discards stale notifications when the current control ticket no
longer matches.

Acquire/release ordering is mandatory around state transitions. Unknown
states, layout mismatch, generation mismatch, ticket regression, out-of-range
length, or mutation of a `READING` slot poisons the process generation.

## Backpressure

- Video may replace a `READY` slot only through the existing GOP-safe policy.
  Losing a dependency chain enters drop-until-IDR, requests a keyframe, and is
  bounded by the existing recovery deadline.
- Audio replaces the oldest `READY` packet. It never waits on video and never
  overwrites `READING` data.
- If every slot is `READING`, the producer drops the current real-time packet;
  it does not allocate, block control, or widen the ring.
- Control events remain on the bounded JSON control lane because they are
  sparse lifecycle/proof/stat events, not the media hot path.

## Lifecycle barriers

- Mapping identity and the existing session nonce are bound to exactly one
  process generation.
- `prepared` proves that mappings and native capture are ready; media slots may
  not publish before `BeginMedia`.
- Reconfigure increments the existing media gate. Old-gate notifications and
  slots are discarded before resume.
- Rebind quiesces and awaits WebRTC writers before committing the replacement
  session binding. A new process receives new mappings.
- Stop/cancel/timeout closes notifications, discards queued slots, waits for
  readers/writers, then reaps the helper. Crash cleanup releases mappings when
  the last daemon ingress lease is dropped; WebRTC-owned detached bytes do not
  retain the mapping.

## Platform projection

Unix uses anonymous/unlinked file-backed mappings plus inherited notification
descriptors. Windows uses the same layout and state machine through
generation-scoped file mappings and independent named notification pipes. The
Windows production WebRTC dispatch no longer selects the daemon-local recorder
or payload-pipe compatibility path.

## Acceptance evidence

Implementation is not complete until evidence proves all of the following:

1. Normal media notification descriptors carry no H.264/Opus payload bytes.
2. Daemon payload reception validates in place, performs exactly one bounded
   ownership-detach copy, and releases the shared slot before WebRTC/NACK can
   retain the sample.
3. Resident memory remains within the negotiated ring bound under a stalled
   receiver.
4. Slow audio cannot delay video/control and slow video cannot delay audio.
5. Slot overwrite, stale ticket, generation mismatch, crash, EOF, reconfigure,
   rebind and shutdown mutations fail deterministically.
6. A comparative live benchmark records payload throughput, CPU, allocations,
   p50/p95/p99 capture-to-WebRTC-write latency and drops for pipe v1 versus
   shared-lane v2 on the same encoded fixture.

## Current implementation evidence

The Unix media-host path now publishes fixed `RVID`/`RAUD` frames into the
generation-owned mapping and writes only a 56-byte notification ticket. The
daemon claims the slot, decodes the fixed header into allocation-free compact
media metadata, and validates the mapped payload through `Bytes::from_owner`.
It then copies the validated payload once into transport-owned `Bytes` and
releases the mapped frame immediately. This prevents the RTP packetizer or NACK
history from pinning a one-to-three-slot producer ring for network lifetime.

`VideoConfig.max_nal_unit_bytes` binds the encoder to the packet transport. The
current daemon contract uses 1160 bytes, below rtc/webrtc's 1188-byte H.264 RTP
payload budget. OpenH264 and VideoToolbox receive this exact bound, and the
daemon rejects a larger emitted NAL before packetization. This avoids FU-A
linearization for ordinary NALs, but the WebRTC sample is intentionally backed
by detached transport memory rather than the mapped slot. Small SPS/PPS
aggregation and downstream RTP serialization/SRTP remain separate costs.

On macOS, VideoToolbox AVCC output and SPS/PPS are appended directly into one
owned Annex-B access-unit buffer. The callback borrows a contiguous CoreMedia
block while rewriting length prefixes; non-contiguous blocks alone use a
bounded linearization temporary. The resulting owned access unit is then
copied once into the shared slot, preserving the lease boundary.

`plugins/remote-desktop/native-protocol/tests/shared_media_lane_benchmark.rs`
compares this path with the former payload-pipe framing using the same encoded
fixture. Both sides now cross the canonical generation, sequence, media-gate
and H.264 conversation validator; shared v2 then detaches transport-owned bytes
and proves the producer can reuse the slot while those bytes remain alive.
Allocation and throughput numbers are comparative evidence, not an end-to-end
zero-copy claim.

`tools/scripts/benchmark-remoteapp-shared-media-lane.sh` records allocation
volume, throughput and p50/p95/p99 hot-path latency. Allocation and bounded-
regression invariants are gates; one-run latency superiority is not, because
scheduler noise makes that claim non-reproducible. This remains a same-process
validated media-path benchmark: it does not satisfy acceptance item 6 by
itself and must not be presented as live capture-to-RTP/SRTP or cross-device
evidence.

`tools/scripts/host-remoteapp-media-host-e2e.sh` now passes in a Linux/X11
container with two real process-owned windows. The process test discovers the
windows through EWMH PID and application identity, starts exact window and
application capture generations, consumes H.264 through the same shared
mapping and 56-byte notification path, crosses the reconfiguration barrier,
and proves typed invalidation when application membership changes. It proves
the real local capture-to-daemon carrier, but not RTP/SRTP transmission to a
second device.

The Windows source path creates two generation-scoped named mappings and two
independent named notification pipes, opens producer/consumer views with
different access rights, binds the helper to a kill-on-close Job, and selects
the hosted media path from production WebRTC dispatch. The native-host,
media-host, protocol and root daemon/plugin graphs cross-compile for
`x86_64-pc-windows-gnu`. This is implementation evidence only: it does not
prove a live interactive Windows desktop, WGC behavior, RTP rendering or crash
recovery on a real Windows host.
