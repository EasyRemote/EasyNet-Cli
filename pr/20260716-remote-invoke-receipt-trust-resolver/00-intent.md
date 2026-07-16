# Intent

Remote `Invoke` forwarding already verifies finalized admission and terminal
receipts, but the standalone remote invocation helper used the local key-service
resolver. That resolver is appropriate for local daemon client projections, not
for authenticating remote callee or host receipt signatures.

This slice gives the standalone remote path the same trust boundary as daemon
dispatch: receipt verification resolves keys from the daemon realm trust anchor.
