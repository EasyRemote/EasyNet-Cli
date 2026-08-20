# Architecture

`easynet_sdk.axon_addressing` is a canonical runtime SDK module. Its kind
projection helpers translate Axon canonical vocabulary into stable SDK facade
vocabulary, so they should be named as runtime projection helpers.

This slice renames only private helpers:

- `_product_ura_kind` -> `_runtime_ura_kind`
- `_product_ability_owner_kind` -> `_runtime_ability_owner_kind`

The SPEC v2 gate is extended so product-shaped Addressing helper names cannot
return to the SDK root.
