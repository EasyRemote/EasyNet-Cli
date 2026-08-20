# Daemon Resolver Latest Input Boundary Plan

## Objective

Remove lower-layer daemon resolver input alias acceptance so `namespace.resolve`
and proxy-resolve paths require the latest daemon wire fields after SDK request
DTOs have been translated.

## Current Defect

The typed daemon wrapper already rejects `query_name` as an unknown
`namespace.proxy_resolve` field, but `DaemonRouteResolver::resolve_query_json`
still read alternate snake-case keys such as `query_name`, `ability_name`, and
`realm_hint`, plus `qType`. A direct resolver caller could therefore bypass the
latest-only daemon input contract.

## Steps

1. Keep SDK request DTOs snake_case and keep the Directory SDK carrier lowering
   to daemon `namespace.resolve` wire fields.
2. Make the daemon resolver read only latest daemon wire fields:
   `queryName`, `abilityName`, `realmHint`, and `qtype`.
3. Add a resolver unit test proving retired snake-case keys are not accepted as
   equivalent input.
4. Extend the daemon latest-input boundary gate to reject this fallback pattern.
