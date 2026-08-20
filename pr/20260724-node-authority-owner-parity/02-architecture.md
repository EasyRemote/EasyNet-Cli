# Architecture

Layering:
- SDK authority facade owns typed authority/request shape validation.
- Authority transport owns provider-specific minting.
- Daemon admission owns runtime proof enforcement.

Boundary decision:
- Node should accept and expose `session_owner_ura` and `creator_principal_ura` as canonical SDK facts.
- The Node staged transport payload should omit those fields when the current provider wire does not consume them, matching Go/Python staging behavior.
