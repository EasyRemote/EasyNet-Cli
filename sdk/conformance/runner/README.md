# Conformance Runner

This directory defines the runner contract for language SDK conformance.

The repository ships a manifest runner as `cargo run -p
sdk-conformance-runner`. It loads the shared cases from `../cases`, validates
fixture and schema references, validates every referenced fixture through
`../fixture-schema-bindings.json`, and emits machine-readable result records.
This is the common integrity gate for every language implementation.

`fixture-schema-bindings.json` is the closed fixture contract. Every
`../fixtures/*.v4.json` file must appear exactly once and point at one
`../../schemas/*.schema.json` file. The runner rejects missing bindings,
duplicate bindings, missing fixtures, missing schemas, and fixture payloads that
do not satisfy the bound schema before it considers a language runtime-conformance
report.

Each SDK facade may provide its own runtime conformance report, but it must consume
the same cases from `../cases`, the same fixtures from `../fixtures`, and emit
equivalent machine-readable results. Pass the report with `--conformance-report` to
turn manifest validation into a language runtime-conformance gate:

```bash
cargo run -p sdk-conformance-runner -- \
  --language rust \
  --conformance-report sdk/conformance/runner/rust-runtime-conformance-report.json

cargo run -p sdk-conformance-runner -- \
  --language c_abi \
  --conformance-report sdk/conformance/runner/c-abi-runtime-conformance-report.json

cargo run -p sdk-conformance-runner -- \
  --language go \
  --conformance-report sdk/conformance/runner/go-runtime-conformance-report.json

cargo run -p sdk-conformance-runner -- \
  --language python \
  --conformance-report sdk/conformance/runner/python-runtime-conformance-report.json

cargo run -p sdk-conformance-runner -- \
  --language node \
  --conformance-report sdk/conformance/runner/node-runtime-conformance-report.json

cargo run -p sdk-conformance-runner -- \
  --language java \
  --conformance-report sdk/conformance/runner/java-runtime-conformance-report.json

cargo run -p sdk-conformance-runner -- \
  --language swift \
  --conformance-report sdk/conformance/runner/swift-runtime-conformance-report.json
```

Minimum live result record:

```json
{
  "case_id": "invocation/complete_tuple",
  "language": "go",
  "profile": "runtime_core",
  "case_sha256": "...",
  "selector": "TestInvocationBuilderBuildsCompleteTuple",
  "evidence": [{
    "kind": "go_test",
    "ref_path": "sdk/go/invocation_test.go",
    "sha256": "..."
  }],
  "collected_tests": ["TestInvocationBuilderBuildsCompleteTuple"],
  "attestation_sha256": "...",
  "status": "passed",
  "error_code": null,
  "executions": [
    {
      "phase": "execution",
      "command": ["go", "test", "-json", "-run", "^TestInvocationBuilderBuildsCompleteTuple$", "-count=1", "./..."],
      "working_directory": "sdk/go",
      "exit_code": 0,
      "output_sha256": "..."
    }
  ]
}
```

Skipped required cases block a `language-stable` claim.

Runtime conformance reports are schema-v2 coverage manifests, not test-result reports. They
contain no `status`; the schema rejects a committed status attestation. Each
record maps a shared case to a test source and pins that source by SHA-256.
`execution-manifest.json` is runner-owned and binds that same case to one exact
selector and evidence path. The runner verifies the case digest and evidence
hash, proves the selector is declared in the bound source, collects the
selector through the language test tool, then executes that exact collected
test. Reports cannot supply or override selectors or commands.

A required case without a report record fails as `CONFORMANCE_REPORT_MISSING`; a
mismatched profile, invalid evidence scope, missing evidence, stale hash, or
report/manifest evidence mismatch fails closed. Missing execution is
`CONFORMANCE_REPORT_EXECUTION_MISSING`; an uncollected, multiply collected,
unrelated, or failing selector is `CONFORMANCE_REPORT_EXECUTION_FAILED`. The
emitted case SHA-256, evidence SHA-256, selector, collected test, command,
working directory, exit code and command-output SHA-256 form one live result.
The runner hashes those fields into `attestation_sha256`, so replacing any one
of them produces a different attestation.

The report is closed over the shared manifest. Every record must match an
existing manifest case and that case must declare the requested language in
`required_for`; unknown or language-undeclared records invalidate the report
instead of being ignored. Evidence kind must match the report language, for
example `rust_test`, `c_abi_test`, `go_test`, or `python_test`; cross-language
evidence is rejected. Evidence paths must also fall under the test-source roots
covered by that language's fixed suite, such as `sdk/go/**/*_test.go` or
`sdk/python/tests/test_*.py`.

Every language implementation consumes the generic runtime cases explicitly listed for its
language in `required_for`. The shared manifest contains no product profile
cases. Product repositories own their workflow tests and prove that their local
facades lower to generic Invocation, Addressing, stream, bidi, health and
authority interfaces. Inline samples may remain as focused unit tests, but they
do not replace the shared case-aware parity gate.

The Go conformance runner collects and executes its fixed suite with the
`runtime_direct` build tag because that runtime-direct provider path is an
explicit part of the Go runtime evidence set. The tag is applied identically to
collection and execution and is included in each command attestation.
