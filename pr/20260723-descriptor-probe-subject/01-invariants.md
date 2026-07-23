# Invariants

- Descriptor catalog probes never synthesize a remote target ability descriptor.
- Remote descriptor probes require an explicit caller URA and caller signer
  before any daemon I/O.
- Probe subject selection is a closed owner-kind state machine:
  - Device callee: subject is the device callee URA.
  - Authority/Hub callee: subject is the meta ability URA.
  - Any other callee kind: fail closed before route lookup.
- No helper named as target-owned or fallback subject derivation remains in the
  FFI descriptor resolver path.
