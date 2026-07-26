# Intent

## Goal

Remove a legacy/compatibility acceptance path in the federation resolver
configuration. `ResolverConfig` must reject unknown fields instead of silently
dropping them and continuing with a degraded route model.

## Non-goals

- Change realm resolution semantics for valid current configuration.
- Add a migration or fallback path for stale resolver config.
- Add product-specific resolver fields.

## Acceptance criteria

- Current `easynet_rendezvous` and `static_hubs` config continues to deserialize.
- Unknown top-level resolver config fields fail closed with a typed serde error.
- Resolver tests prove bare-realm and unknown-field inputs do not fall back to
  local or endpointless routing.
