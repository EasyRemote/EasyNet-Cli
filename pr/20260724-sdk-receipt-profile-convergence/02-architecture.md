# Architecture

The SDK defines a product-neutral canonical runtime model. Receipt proof facts
bind caller, callee, subject, signer, subject_ref, and authority_proof issuer
through runtime identity profiles.

Allowing a local legacy profile whitelist inside individual language SDKs
creates three problems:

1. Receipt canonicalization no longer has one shared identity profile model.
2. Java/Node/Swift can accept receipts that Go/Python reject.
3. Product compatibility terms become indistinguishable from runtime proof
   evidence.

The correct boundary is a narrow local predicate equivalent to the canonical
profile parser currently used by Go/Python: `axon-strict-v2` is accepted and all
other profiles fail closed.
