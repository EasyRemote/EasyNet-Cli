# API Contract

No public Go API shape changes.

`CABIPublicationTransport.EnableAbilityImpl(ctx, requestJSON)`:

1. Requires an open C ABI handle.
2. Calls `easynet_publication_build_enable_ability_impl_invocation`.
3. Submits the returned carrier with `easynet_invocation_invoke`.
4. Extracts `output_json`.
5. Calls `easynet_publication_project_enable_ability_impl_result`.

`DisableAbilityImpl` follows the same path with disable symbols.

Errors are the existing typed C ABI/profile errors; no compatibility fallback is
introduced.
