# SDK Conformance Suite

The conformance suite is the language-neutral behavior contract for the
EasyNet Daemon SDK. It prevents language facades from redefining daemon or Axon
semantics while still allowing idiomatic APIs.

## Layout

```text
sdk/conformance/
  cases/
    *.yaml
  fixtures/
    *.json
  runner/
    README.md
```

Cases are declarative. Fixtures are golden DTO payloads validated against
`sdk/schemas`.

## Case Format

Each YAML case uses:

```yaml
id: invocation/complete_tuple
profile: runtime_core
required_for:
  - rust
  - c_abi
steps:
  - action: build_invocation
    fixture: invocation.complete.v4.json
expect:
  result: ok
```

Runners may translate actions to idiomatic language APIs, but they must report
the same case id, profile, result, and typed error code.

## Runner Contract

A language runner must:

- Load case YAML from `sdk/conformance/cases`.
- Load fixture JSON from `sdk/conformance/fixtures`.
- Validate fixture files against `sdk/schemas` before executing behavior.
- Fail when a public API exposes raw Axon/proto/runtime types.
- Treat skipped cases as instability evidence unless the profile is undeclared
  for that language.
- Emit machine-readable results with `case_id`, `language`, `profile`,
  `status`, and `error_code`.

Go and Python facade tests must consume shared cases from
`sdk/conformance/cases` and shared fixtures from `sdk/conformance/fixtures` for
shipped local DTO/actions and projection-only profile behavior, including
Runtime Core, Directory + Identity, Mission, Admin + Gateway, Publication,
Events, Surface, Compatibility, Receipt, Host Binding, and Wrapper profile
adapters. Inline samples may remain as focused unit tests, but they do not
replace the shared case-aware parity gate.

## Minimum Commands

```text
cargo run --bin sdk-conformance-runner -- --language rust --format jsonl
cargo run --bin sdk-conformance-runner -- --language c_abi --format jsonl
cargo test --lib --features axon-pb sdk_
cargo test --lib --features axon-pb ffi::
bash tools/scripts/check-sdk-scaffold.sh
```

Future runners should add:

```text
cd sdk/go && go test ./...
cd sdk/python && python3 -m unittest discover -s tests
npm test --workspace sdk/node
./gradlew :sdk:java:test
swift test --package-path sdk/swift
```

## Required Case Families

- version and ABI compatibility
- daemon lifecycle and degraded readiness
- complete Invocation tuple
- invocation builder handle state transitions
- invocation handle terminal monotonicity
- canonical material delegated to Axon
- typed error JSON projection
- prepared-not-submittable
- pre-signed submit
- local daemon signing boundary
- terminal monotonicity
- authority mutual exclusion
- stream close and bidi close-send lifecycle ownership
- stream terminal ordering and backpressure
- bidi frame0 and close-send behavior
- directory read-model carriers, resolve carrier/projection, page projection,
  pagination, and no-default-fanout
- identity URA and DescriptorRef projection delegates to Axon helpers
- receipt fetch/project/verify/causal-ref
- receipt projection never upgrades summary-only data to verified
- publication ResourceRef, package validation, and complete Invocation carriers
- host binding frame codec and output-hash folding
- mission run/track/cancel complete Invocation carriers and MissionStatus
  projection
- events directory subscription carrier, explicit cursor projection,
  dropped-event reports, and terminal frames
- admin/gateway agent/session carriers, lifecycle readiness flags, agent record
  projection, and lifecycle result projection
- compatibility OpenAI model/chat carriers, canonical model id validation,
  unary completion projection, and stream envelope projection
- convenience wrapper file/session/media record projections without execution
  transport ownership
- profile ownership exclusivity

The scaffold in `cases/` names the first shared cases. A profile must add its
full case set before it can be marked profile-ready.
