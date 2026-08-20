# Invariants

1. CLI command modules do not construct `RemoteAbilityInvocationTarget` for
   target-owned system abilities.
2. CLI command modules do not call the low-level `invoke_remote_target` helper
   for simple device/hub-owned system ability sugar.
3. Descriptor-bound `ability invoke --node` remains outside this slice because
   it preserves explicit origin-proof and subject semantics.
4. The no-`axon-pb` public behavior remains unchanged: device-targeted remote
   calls fail through the existing federation-not-wired error, and voice
   signaling falls back to local dispatch.
5. Public CLI payload shape and response handling remain unchanged.
