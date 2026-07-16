# SDK Capability Parity

The runtime SDK has one architecture. Rust, C ABI, Go, Python, Node, Java and
Swift are implementations of the same capability model, not independent SDK
designs. The machine-readable concept source is
[`conformance/canonical-public-api.json`](conformance/canonical-public-api.json);
[`conformance/sdk-parity-matrix.json`](conformance/sdk-parity-matrix.json) is
the generated seven-language Cartesian product consumed by parity gates. This
file explains how to read that pair.

## State model

| State | Required evidence |
| --- | --- |
| `unsupported` | no public shipped capability |
| `seam` | public interface and lifecycle exist; no shipped provider claim |
| `provider-backed` | explicit production provider plus runner-owned executable conformance evidence |
| `cutover-ready` | first-class consumers use it and obsolete lower/product layers are deleted |

A placeholder type, default implementation, fallback provider, committed report
status or documentation claim is not evidence.

## Canonical capabilities

The matrix may contain only product-neutral capabilities from these families:

| Family | Examples |
| --- | --- |
| runtime discovery | ABI/version and capability discovery |
| lifecycle | environment, native runtime, runtime host and runtime handle state |
| Addressing | URA, descriptor reference and subject projection delegated to Axon |
| Invocation | complete draft, prepare/sign/submit, result and handle lifecycle |
| transport | unary, server stream and bidi |
| authority | canonical authority metadata projection/materialization |
| identity material | runtime identity, public projection and sign-only capability |
| managed signing | subject-bound create/list/rotate/revoke/expiry through key-service |
| access control | product-neutral grants, decisions and authority proofs over daemon governance abilities |
| principal lifecycle | lifecycle aggregate, enrollment, public-key binding, recovery, suspension and grants; each sub-capability remains separately represented in the matrix until provider-backed |
| Directory | canonical resolve and subscription, not product dashboard rows |
| receipts | verifiable receipt/causal facts and continuation references |
| runtime events | bounded streams, typed cursors and resume semantics |
| runtime administration | product-neutral daemon/runtime lifecycle and readiness control |
| health | runtime readiness and diagnostics |
| errors | shared code/class/retry semantics |

Product Directory pages, hosted-agent workflow, Mission, publication, pages,
OpenAI compatibility, host binding, product events, wrappers and companion
lifecycle are not capability rows. They are downstream product behavior over
the generic capabilities above.

## Language requirements

### Go and Python

- expose the same capability identifiers and state meanings;
- validate the same lifecycle and tuple invariants;
- delegate Addressing to the same Axon grammar;
- use only generic C ABI v5 when native;
- cite language-specific tests for each non-unsupported state;
- do not publish product workflow modules, parallel canonical models or
  compatibility aliases that preserve a second provider/state machine.

### Node, Java and Swift

These packages may expose a smaller subset. They must implement the same
semantics for any exported concept and leave missing concepts unsupported.
Publishing a product DTO/client merely to make a language look symmetric is a
parity failure.

## Current verification

The aggregate gates validate:

- matrix schema and evidence references;
- complete seven-language matrix closure over the canonical concept schema;
- Go/Python capability-set equality where both languages expose the mature
  provider surface;
- complete Invocation conformance;
- Addressing accepted/rejected vector parity;
- lifecycle monotonicity and close ownership;
- exact ABI v5 exports;
- absence of product SDK surfaces and legacy identity names.

Consumer readiness is proven in the consumer repository:

- EasyNet backend owns its Go product ports and DTOs;
- EasyRemote owns its Python product workflows and projections;
- each consumer depends on generic Runtime/Addressing/stream/bidi/health
  interfaces without copying Axon protocol logic.

## Package metadata

Package metadata is machine-checked across Go, Python, Node, Java and Swift.
That check proves coordinates and build inputs only; publish/release stability
and external registry availability remain incomplete until a release pipeline
proves them.

## Updating the matrix

1. Change the provider/lifecycle implementation.
2. Add executable conformance evidence in both languages where applicable.
3. Update the canonical public API concept schema and regenerate the parity
   matrix.
4. Run the parity, package, runner-owned conformance and product-neutrality
   gates.
5. Do not promote a state until the evidence is present in the same change.
