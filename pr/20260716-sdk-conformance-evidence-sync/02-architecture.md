# Architecture

`sdk/conformance/runner/*-action-adapter-report.json` owns the mapping from
language/case to executable source evidence.

`sdk/conformance/canonical-public-api.json` owns public capability state,
provider proofs, and source hashes.

`sdk/conformance/sdk-parity-matrix.json` is generated from that canonical model
and must not become an independent truth table.

The validator remains the enforcement boundary: it recomputes evidence hashes
from the current tree and rejects stale reports before considering a result
usable as parity proof.
