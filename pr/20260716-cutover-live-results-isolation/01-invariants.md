# Invariants

1. Live parity evidence must be produced by the same cutover run that consumes
   it.
2. A partial language-slice artifact from an earlier command must not satisfy
   the cutover readiness gate.
3. All language result records in one cutover run must share one run nonce and
   one source-tree attestation.
4. Snapshot source attestations are acceptable only when explicitly opted into
   by the release gate.
5. Generated live-result artifacts remain outside source control.
6. A failing conformance producer may not be hidden by stale artifacts from a
   previous successful run.
