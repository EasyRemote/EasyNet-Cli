# Mission Boundary Proof

## Runtime Ownership
- MissionClient is a generic SDK facade over daemon-owned Mission/EAL runtime operations.
- The SDK builds and forwards complete Invocation carrier requests; the daemon owns execution, persistence, and receipt anchoring.
- MissionStatus and MissionEventPage are typed projections of daemon facts and do not fabricate parent or child receipt references.

## Product Boundary
- No EasyNet-specific, EasyRemote-specific, Pipeline, or product lifecycle concepts are introduced in the Java or Swift SDKs.
- Public DTOs use generic runtime names: MissionCarrierBase, MissionRunRequest, MissionTrackRequest, MissionCancelRequest, MissionEventsRequest, MissionStatus, MissionEventPage, MissionEventStream.

## Stream Boundary
- Live Mission events use the existing Runtime Core stream lifecycle. Mission only projects stream payload JSON into MissionEvent.

## Architecture State
- Java Mission seam: seam -> cutover-ready after provider transport binding.
- Swift Mission seam: seam -> cutover-ready after provider transport binding.
