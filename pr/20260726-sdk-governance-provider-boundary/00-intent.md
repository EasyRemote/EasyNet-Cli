# Intent

Remove the remaining SDK-side generic ability invocation path for runtime governance reads.

`RuntimeAbilityClient` already models catalogue and receipt reads as provider-backed capabilities, but Python `AbilityInvocationClient` can still resolve an arbitrary governance `ability_ura` through the generic descriptor resolver without declaring a provider. That recreates the product failure mode where `meta.list_abilities` and `invocation.history.*` fall into generic route/signer resolution instead of their typed providers.

This iteration makes governance-read classification shared inside the SDK and makes the generic ability invocation facade fail closed for those abilities.
