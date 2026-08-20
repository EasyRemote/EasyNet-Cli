# Canonical Runtime Convergence V2 Closure

Date: 2026-07-18

Status: Withdrawn. This report is not acceptance evidence.

Normative scope:
`docs/spec/canonical-runtime-convergence-v2.md`.

This report records a failed closure attempt. Its source revisions, test
counts, and root-fork claims were invalidated by inspection of the checkout:
product protocol authority and a duplicate admission model remained in Axon,
Java receipts still permitted mutable/default proof facts, downstream public
ingress still derived Invocation tuple members, and the recorded source
revisions were stale. Everything below is retained as rejected historical
evidence only.

A root fork is closed only when callers use the canonical path, the replaced
authority is deleted, automated gates reject reintroduction, and
revision-pinned cross-language or cross-repository behavior is verified.
Adding a preferred path beside a legacy path is not closure.

## Target Architecture

Axon owns the generic canonical runtime model:

- complete Invocation tuples and URA canonicalization;
- descriptor binding, proof, signature, replay, and admission;
- the `LocalRuntime` lifecycle state machine for unary, stream, bidi, child
  invocation, cancellation, deadlines, recovery, and terminal receipts;
- mandatory receipt proof facts and canonical verification; and
- one capability matrix and conformance contract shared by all language
  implementations.

EasyNet-Cli owns downstream product and host policy:

- daemon lifecycle, local key custody, device and Hub policy;
- Mission/EAL, plugins, MCP, media, pages, scheduling, and product providers;
- route classification and complete tuple input; and
- read-only projection of Axon-owned terminal receipts and events.

The backend and EasyRemote consume the SDK through downstream provider
boundaries. They do not own proof, lifecycle, receipt, or canonical wire models.

## Rejected Closure Claims

| ID | Closure evidence |
| --- | --- |
| RF-1 | Product protocols, presets, facades, and product package identities were removed from Axon canonical packages. Generic `AbilityDescriptor`, `AbilityImpl`, provider, content, and bidi primitives remain. Product package, public-surface, downstream consumer, and neutrality gates pass. |
| RF-2 | Mission proto, service, state, runtime, and public SDK facades were removed from Axon. EasyNet-Cli owns Mission/EAL and dispatches complete child Invocations with causal parentage through descriptor-bound `LocalRuntime`. |
| RF-3 | Descriptor-bound requests are the sole public admission input. Plain signing, verification, admission, duplicate Rust domain, and legacy vectors were deleted. Public API manifests, Dendrite exports, and single-authority scans pass. |
| RF-4 | Rust, Go, Python, Node, Java, and Swift consume one lifecycle contract. The shared suite reports 72 of 72 cases passed at `CutoverReady`; the negative contract suite reports 35 of 35 assertions passed. |
| RF-5 | Explicit caller signers or daemon `KeyService` custody are required. Generated and cached process-local authority fallbacks were removed, and key-custody boundary gates pass across CLI and downstream repositories. |
| RF-6 | Receipt constructors require complete authority, descriptor, implementation, environment, input/output, parent, and signature facts. Default/empty compatibility constructors were removed; canonical verifier and receipt boundary suites pass. |
| RF-7 | The daemon route inventory requires unary, stream, bidi, loopback, and exact ability routes to enter descriptor-bound `LocalRuntime`. Direct product receipt/finalization authorities were deleted; migration and live-daemon gates pass. |
| RF-8 | Caller, callee, ability, subject, nonce, causal context, and args are explicit at SDK, FFI, daemon, and backend boundaries. Public-ingress defaulting was removed and complete-tuple gates pass. |
| RF-9 | Active source and normative text use URA. Axon owns the only editable proto source; generated copies are deterministic and byte-for-byte checked in both repositories. |

## Stale Source Evidence

Previously claimed Axon source revision:
`43cec35329fe51a300f1eb0b7476eb78d62d698b`.

Principal Axon commits:

- `a3256e94f48aa3d2f1afebeabf90dd81514ff2d0` -
  Converge Axon on the canonical runtime architecture.
- `afca033f2e113e9d998efc7578cb028cea5ed4eb` -
  Publish reproducible canonical runtime benchmark evidence.
- `d9c3daeae5701e9767075b273b30f390f1e14b18` -
  Close canonical receipt verification and lifecycle construction gaps.
- `a834741b32179183f7746d2addb66eb477a04211` -
  Refresh canonical runtime benchmark evidence after lifecycle refactor.
- `43cec35329fe51a300f1eb0b7476eb78d62d698b` -
  Refresh Rust lifecycle convergence evidence.

Principal EasyNet-Cli commits:

- `af57b6c38b8ad5c9bd4f593ecae423dd86b0d041` -
  Regenerate CLI protobuf surfaces from canonical Axon schema.
- `bcedbc42a5b52431b6edb3975f6d676be7f04b24` -
  refactor: converge daemon on canonical runtime.
- `3f85feada9743f732ac9d996ff710b88b66114fd` -
  refactor: converge SDK facades on canonical runtime.
- `da3fe0046ab9138f764a4d63b648cdf6ce1accd1` -
  Enforce released edge-adapter removal and caller policy.
- `2b2ddee7083c48343e21a0fb7c595958e3ddf03e` -
  Bind canonical public API evidence to stable Axon source.
- `61ef31b5232413439fdeae921b581c59c88c1902` -
  Stabilize bounded Axon source attestations.

The source revision is the most recent commit touching the bounded Axon
`sdk`, `core/proto`, or Dendrite public-header roots. Documentation-only commits
cannot invalidate source evidence. Dirty bounded roots are attested by a
deterministic content hash and cannot reuse committed evidence.

## Stale Verification Evidence

Axon:

- Rust canonical SDK: 229 tests passed with all features.
- Axon runtime: 296 tests passed with all features.
- Canonical receipt verifier: 23 tests passed.
- Dendrite bridge: 64 unit tests and 4 integration tests passed.
- Python: 177 passed, 3 skipped.
- Go: 261 top-level tests and 42 subtests passed, 3 skipped.
- Node: 75 passed, 3 skipped; 16 industrial and 14 axiom cases passed.
- Java: 133 tests passed.
- Swift: 128 tests passed.
- Six-language lifecycle: 72 of 72 transition/recovery cases passed against
  semantic contract SHA-256
  `6c6578861ba793be819b5ec8f97de9257900eb91b020883aab24d04c838d79e7`.
- Lifecycle fail-closed suite: 35 of 35 assertions passed.
- Product-neutral package, public-surface, receipt-proof, Mission-boundary,
  URA, deterministic proto-source, Dendrite ABI, and single-Rust-authority
  gates passed.

EasyNet-Cli and downstream:

- `cargo test --all-targets`: 4,059 library tests passed, 0 failed, 3 ignored;
  all integration, script-contract, and example targets passed.
- `check-sdk-cutover-readiness.sh --self-test`: passed.
- `check-sdk-cutover-readiness.sh`: passed from a committed source state,
  including canonical public API generation, released edge-adapter policy,
  product neutrality, seven-language live conformance and parity, ABI,
  package metadata, URA, receipt, daemon migration, release packaging,
  downstream boundaries, key custody, product smokes, runtime events, live
  daemon E2E, and Go/Python live smokes.
- EasyRemote product smoke: 308 passed, 4 skipped.
- EasyNet backend: `go test ./...` passed.

## Benchmark Evidence

`EasyNet-Axon/sdk/rust/benches/baseline-v2.json` is the measured canonical
LocalRuntime V2 baseline. Its validator checks source, harness, executable,
workload, raw samples, and cleanup facts. It covers unary, stream, bidi,
cancellation, allocations, and a 64-request bounded-concurrency scenario. The
bounded scenario observed at most 16 in flight, and every scenario ended with
zero active invocations.

No percentage performance claim is used as acceptance evidence.

## Claimed Compatibility Boundary

Public behavior remains compatible only through the released Go and Python
edge adapters enumerated by
`sdk/conformance/edge-adapter-policy.v1.json`. Their public shapes and caller
inventory are frozen, new definitions and new callers are rejected, and their
declared removal version is `1.0.0`. They construct complete canonical requests
and do not retain proof, signer, lifecycle, or receipt authority.

## Historical Records

`architecture-convergence-audit-2026-07-14.md`,
`open-fix-delegation-spec-2026-07-17.md`, and this report are historical
records. None describes an accepted closure checkout. Live status is maintained
in Section 12 of `docs/spec/canonical-runtime-convergence-v2.md`.
