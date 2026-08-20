Current state:
- SDKs expose signer handle projection and Runtime Core signer/provider primitives.
- Product callers can still compose signer workflows by separately acquiring a handle and constructing a signer.

Gap:
- The canonical SDK facade does not expose a single acquisition step that proves the handle came from the daemon identity profile and is provider-bound before signing.
