# Architecture

## Root abstraction problem

The Go SDK still exposes `sdk/go/provider/easynet` as an allowed provider owner.
That package contains only lifecycle forwarding and a product credential identity
adapter. The forwarding code is generic runtime-host provider behavior; the
identity adapter translates EasyNet daemon credentials into canonical runtime
identity and should be owned downstream.

## Target model

- `sdk/go/provider/runtime` owns runtime host lifecycle DTOs and the lifecycle
  facade over `RuntimeHost`.
- `RuntimeHost` remains the canonical state-machine owner.
- Product credentials are not projected by the SDK.
- The conformance model treats the Go provider as `runtime_provider`, matching
  Python's converged ownership.

## Boundary

This iteration changes Go SDK provider ownership only. Product e2e route
terminality and remote bidi cutover remain separate runtime/product ingress
items.
