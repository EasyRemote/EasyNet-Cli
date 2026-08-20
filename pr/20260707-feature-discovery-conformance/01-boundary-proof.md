# Boundary Proof

## Ownership

Feature discovery is part of the SDK process root and Runtime Core. It tells
bindings which generic SDK capabilities are present before they open runtime
traffic.

## Product Boundary

The feature catalog contains runtime profiles and symbols only. It does not
advertise EasyNet product routes, EasyRemote host lifecycle, backend account
state, UI health, or product-specific receipt semantics.

## Canonical Model

All language facades decode the same `feature-discovery.schema.json` DTO and
the same `feature-discovery.v4.json` fixture. Language-specific clients may
present idiomatic objects, but they must not maintain independent feature
catalogs.

## URA Discipline

The slice does not introduce address terminology or URI aliases. It only
describes generic SDK profile and symbol availability.
