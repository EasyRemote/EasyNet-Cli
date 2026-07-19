# Daemon Bidi Route Inventory

Converge daemon `InvokeBidi` exact-route ownership with the existing unary and
server-stream route model.

This slice does not claim the long-lived `session.open` carrier is fully
cut over to LocalRuntime-owned data-plane finalization. It removes the route
model fork that kept bidi exact routes as an independent string list in
`BidiDispatcher`, and makes V2 convergence gates consume the same runtime route
inventory guard used by the architecture convergence gate.
