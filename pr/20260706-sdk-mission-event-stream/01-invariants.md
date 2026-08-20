# Invariants

- Mission owns `mission.events` carrier/projection and typed `MissionEvent` decoding.
- Runtime Core owns stream handle lifecycle, ordering, buffering, cancel, close, and terminal state.
- Language facades must not parse daemon/Axon protocol frames beyond SDK stream DTO projection.
- The conformance case must keep Go and Python behavior aligned.
