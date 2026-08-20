# EasyRemote SDK Boundary Gate Plan

## Intent

Add an executable EasyNet-Cli gate that audits the sibling EasyRemote repository
for the daemon SDK boundary. The current SDK already has `ConsumerBoundaryAuditor`
and synthetic conformance fixtures; this task makes the product cutover check
repeatable against real EasyRemote source without moving product behavior into
the SDK.

## Boundary Proof

- Axon remains the owner of URA, DescriptorRef, Invocation, receipt, stream, and
  bidi protocol semantics.
- EasyNet-Cli SDK owns the facade boundary and the audit rule that consumers
  must not import raw Axon, raw C ABI, old private transports, or local protocol
  codecs.
- EasyRemote remains the product/decorator layer. The gate only observes its
  source tree; it does not add EasyRemote product logic to EasyNet-Cli.
- SPEC is not modified.

## Invariants

1. The script must reject raw lower-layer imports, raw FFI/ABI symbols, old
   `_transport` modules, local Invocation JSON codecs, raw URA/DescriptorRef
   helpers, and local receipt/host-stream carrier semantics through the existing
   SDK auditor.
2. The default target is the sibling `../EasyRemote`, overridable by
   `EASYNET_EASYREMOTE_ROOT` or an explicit argument.
3. Missing or non-Python EasyRemote roots fail when auditing a real path.
4. `--self-test` must prove both allowed SDK-only consumers and forbidden raw
   lower-layer consumers.
5. The SDK parity product-boundary ledger must reference this script as static
   evidence, but the capability matrix must not reclassify EasyRemote as an SDK
   profile.

## Implementation

1. Add `tools/scripts/check-easyremote-sdk-boundary.sh`.
2. Run the existing `easynet_sdk.audit_consumer_boundary` over the selected root.
3. Add self-test fixtures for SDK-only and forbidden EasyRemote-like consumers.
4. Reference the script from the EasyRemote product boundary rule in
   `sdk/conformance/sdk-parity-matrix.json`.
5. Ensure scaffold checks include the new gate's self-test.

## Verification

- `tools/scripts/check-easyremote-sdk-boundary.sh --self-test`
- `tools/scripts/check-easyremote-sdk-boundary.sh /Users/macbook.silan.tech/Documents/GitHub/EasyRemote`
- `tools/scripts/check-sdk-parity-matrix.sh --self-test`
- `bash tools/scripts/check-sdk-scaffold.sh`
- Python focused boundary tests
