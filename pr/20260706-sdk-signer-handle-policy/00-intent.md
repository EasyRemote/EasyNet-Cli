# Intent

Close the daemon key-inventory policy gap for SDK signer-handle projection.

Signer handles are only valid when they are projected from daemon identity key
inventory facts. The native/C ABI projection must carry the same provenance and
policy guardrails already required by the Go and Python facades: active key
state, inventory owner, and deterministic daemon inventory policy reference.
