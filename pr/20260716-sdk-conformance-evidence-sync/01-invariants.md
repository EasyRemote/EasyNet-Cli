# Invariants

1. Every executable evidence record references a file inside the repository.
2. Every evidence `sha256` equals the current bytes of its referenced file.
3. The parity validator must stay fail-closed on stale or forged evidence.
4. Matrix state is generated from `canonical-public-api.json`; hand-edited
   matrix rows are not an owner boundary.
5. Existing live results may be accepted only when their source attestation,
   toolchain attestation, Axon revision, execution proof, and evidence hashes
   all match.
