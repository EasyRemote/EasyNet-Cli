# Invariants

1. Runtime host lifecycle is a generic provider capability, not an EasyNet
   daemon lifecycle facade.
2. Product credential adapters do not belong in the canonical SDK.
3. The provider must validate process policy before invoking transport.
4. The canonical runtime lifecycle state machine remains owned by the SDK root.
5. No source-compatible EasyNet provider alias remains after migration.
