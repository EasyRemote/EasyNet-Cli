# RemoteApp cross-device RemoteApp evidence

## Intent

Separate the existing cross-device synthetic carrier smoke from a real
cross-device RemoteApp product session proof.

The existing `remoteapp-cross-device-product-smoke.sh` remains useful: it proves
Hub routing and descriptor-bound synthetic stream/bidi transport across distinct
devices. It explicitly does not prove real OS capture, input policy, frontend
rendering, or RemoteApp session behavior.

This slice adds a dedicated artifact verifier for the missing product proof and
requires it from the top-level product-completion gate.

## Product gap closed

RemoteApp product completion can no longer be satisfied by synthetic
stream/bidi carrier evidence alone. A product-complete claim now also requires a
cross-device RemoteApp artifact with display/window/application sessions,
remote target inventory, rendered media, input policy observation, and terminal
receipts.
