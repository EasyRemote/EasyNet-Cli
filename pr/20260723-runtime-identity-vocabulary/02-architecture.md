# Architecture

The root abstraction issue is vocabulary drift at the daemon identity boundary.
The daemon is a product process, but this code path is part of the canonical
runtime execution model: signer custody, local loopback caller identity,
admission key resolution, and hosted-agent authority facts.

Using "product" at this boundary makes the model look downstream-owned even
though the code is enforcing runtime invariants. This slice renames comments to
runtime-local language and adds a gate so product-shaped phrasing does not
return to these identity modules.
