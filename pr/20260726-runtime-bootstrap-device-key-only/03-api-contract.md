# API Contract

## Request

`runtime.bootstrap_self_identity` keeps the same request fields:

- `realm`
- `node_id`
- `owner_id`
- `public_key_b64`

## Behavior

The request installs the key for `easynet:///r/<realm>/device/<node_id>` only.

## Errors

Resolving the old Agent alias returns `bootstrap_identity_key_not_found:<agent_ura>`.

## Compatibility

No compatibility resolver is retained for the retired Agent alias. Public request compatibility is preserved without preserving internal alias semantics.
