# Invariants

1. SDK consumers must stay above the SDK boundary and must not open runtime-host
   sockets directly.
2. Product runtime binary names may appear only as forbidden detection targets.
3. Direct provider transports remain provider-owned internals, not consumer
   public surface.
4. Diagnostics must use generic runtime-host language.
