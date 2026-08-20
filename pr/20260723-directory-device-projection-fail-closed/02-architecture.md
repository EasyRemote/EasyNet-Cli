# Architecture

The federation directory has two distinct layers:

- Presence registry: liveness state keyed by URA.
- Directory wire projection: canonical cross-realm device directory rows/events.

The removed behavior mixed these layers by allowing arbitrary presence URAs to become directory rows. The new boundary validates canonical device shape before projection and applies remote frames through a fail-closed view update path.
