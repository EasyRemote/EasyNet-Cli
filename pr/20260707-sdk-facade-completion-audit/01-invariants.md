# Invariants

## Boundary Invariants

1. Axon owns URA parsing, DescriptorRef canonicalization, Invocation canonical
   bytes, signatures, admission, receipt verification, stream terminal rules,
   and bidi terminal rules.
2. EasyNet-Cli daemon/Rust/C ABI owns daemon lifecycle, local process policy,
   endpoint discovery, daemon DTO projection, and handle ownership.
3. Go, Python, and Node SDKs are facades. They may map transport, lifecycle,
   errors, async adapters, and idiomatic builders, but must not define a second
   protocol grammar or canonicalization algorithm.
4. Product consumers must not import Axon packages or generated protocol types.
5. Any direct Axon import inside a language SDK must be auditable as a bridge
   to Axon-owned truth and must be protected by an import-boundary test.

## Completion Invariants

1. Runtime Core is not considered complete without lifecycle, health, complete
   Invocation, unary, stream, bidi, terminal observation, typed errors, and
   conformance evidence.
2. EasyRemote cutover remains incomplete if EasyRemote still needs raw FFI,
   raw sessions, Invocation JSON codecs, receipt placeholders, URA builders,
   host-stream codecs, publication/mission/admin transports, or daemon system
   ability carriers.
3. Backend cutover remains incomplete if backend imports Axon/proto/C ABI,
   raw daemon sockets, EasyRemote, subprocesses, or hand-written stream/bidi
   loops for product paths.
4. Profile work must remain in named profile clients. Runtime Core must not
   grow one-method-per-ability product helpers.
