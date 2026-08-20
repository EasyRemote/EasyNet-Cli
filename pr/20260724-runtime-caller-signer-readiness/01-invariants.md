# Invariants

1. Device start readiness requires both daemon capability advertisement and active caller signer custody proof.
2. The proof must use the canonical runtime caller signer resolver; start must not reimplement keyring or managed-user lookup.
3. The proof must verify possession, not just list a key projection.
4. The proof must be product-neutral inside `daemon::identity`; EasyNet-specific start code only selects the active credential User URA.
5. No fallback signer, all-zero user, default local user, or unsigned compatibility path may be introduced.
6. On failure, the runtime projection must not be written as if the daemon were usable.

