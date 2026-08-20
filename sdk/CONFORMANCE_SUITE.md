# Canonical Runtime Conformance Suite

The conformance suite proves that every binding implements the same
product-neutral runtime semantics. It does not standardize product workflows.

## Sources

- `cases/`: declarative semantic requirements;
- `fixtures/`: exact cross-language inputs and projections;
- `sdk-parity-matrix.json`: Go/Python capability state and evidence;
- `runner/`: action adapters and reports;
- language tests: executable provider/binding evidence.

## Required case families

### Addressing

- valid and invalid URA vectors delegate to Axon;
- owner and AbilityDescriptorRef projections are identical across bindings;
- descriptor-bound subjects are explicit;
- malformed structural segments fail closed;
- no binding or product assembles canonical strings independently.

### Complete Invocation

- every seven-tuple slot is required;
- exactly one argument representation is accepted;
- descriptor version survives prepare and dispatch;
- caller, callee, subject, nonce and causal context survive unary, stream, bidi
  and federation relay;
- prepared material is not executable;
- signed material is immutable and submitted at most once.

### Lifecycle

- builder/prepared/signed/handle/stream/bidi transitions are monotonic;
- close is idempotent and observes ownership;
- invalid transitions return typed errors;
- unavailable required providers fail construction or the operation;
- corrupt discovery/catalog state is never projected as an empty success.

### Authority and receipts

- delegation and session-authority metadata are mutually exclusive;
- canonical material is delegated to the authority provider;
- receipt facts used for causal continuation are explicit and opaque;
- summary data alone never claims cryptographic verification.

### ABI v5

- header and export allowlist agree exactly;
- Go/Python native loaders resolve only allowed symbols;
- owned buffers and opaque handles have explicit release operations;
- removed domain symbols and aliases are absent from source, binaries and
  release packages.

### Product neutrality

- public exports contain no product profile client/factory;
- public source contains no product ability literals;
- generic schemas and cases do not encode product DTOs;
- Node/Java/Swift do not ship placeholder product seams;
- downstream products own their DTOs/workflows while importing only generic
  runtime concepts;
- legacy identity spelling is absent where the semantic value is a URA.

## Evidence rules

1. Every non-unsupported matrix state cites an existing executable test.
2. A report generated from a stale source tree is invalid.
3. A case cannot pass through an unimplemented/default action adapter.
4. `provider-backed` requires an explicit provider path, not a DTO-only seam.
5. `cutover-ready` requires deletion of the replaced implementation and an
   import/export boundary gate.
6. Product repository tests may be referenced as downstream evidence, but the
   product contract is not copied into this suite.

## Runner result

An action adapter emits one result per selected case:

```json
{
  "case_id": "invocation/complete_tuple",
  "status": "passed",
  "evidence": ["language test or exact fixture reference"]
}
```

Unknown actions, missing fixtures, missing evidence and schema mismatches fail
the run. Skipping is permitted only when the capability state is explicitly
`unsupported`; a shipped seam/provider cannot silently skip its declared cases.

## Release gate

The SDK release gate runs:

- Rust library check/tests with zero warnings;
- exact ABI/header/package checks;
- Go tests under supported build tags;
- Python tests, typing and import/export audit;
- Node/Java/Swift build/tests for their declared subsets;
- matrix schema/evidence validation;
- product-neutrality, URA naming, project-structure and dead-code guards;
- downstream EasyNet backend and EasyRemote suites after product extraction.

The release is blocked if any current-architecture document, public export or
conformance artifact describes the retired product-profile SDK.
