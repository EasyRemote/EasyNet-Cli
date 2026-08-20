# Verification

Run:

1. `npm test --prefix sdk/node`
2. `bash tools/scripts/check-node-sdk-seam.sh`
3. `bash tools/scripts/check-sdk-ura-naming.sh`
4. `git diff --check`

Acceptance:

1. Node tests prove Directory + Identity delegates to injected transports.
2. Directory list requests apply default and maximum page bounds.
3. Identity rejects whitespace-padded public inputs and projects DescriptorRef
   and URA results without local grammar parsing.
4. Node docs and parity status say `seam`, not `provider-backed`.
