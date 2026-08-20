# Intent: Mission events C ABI runtime execution

Implement `mission.events` for the C ABI backed SDK facade as a daemon-owned
carrier execution path:

- Rust daemon contract builds a complete `mission.events` Invocation carrier.
- C ABI v4 exports the carrier builder alongside existing Mission builders.
- Go and Python C ABI transports execute build -> Runtime Core invoke ->
  MissionEventPage projection.

The SDK must remain a facade over daemon/Axon-owned semantics. It must not parse
Mission storage directly, fabricate receipts, or implement an alternate Mission
runtime.
