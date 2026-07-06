# Invariants

1. Node stream and bidi facades must expose named buffer limits.
2. Facade history retention must be bounded and must never grow with total
   stream lifetime.
3. Overflow must become a terminal typed SDK error projection, not silent data
   loss.
4. Node must not define new daemon frame semantics; terminal detection remains a
   projection over daemon-provided frames.
5. `close` and `cancel` remain explicit lifecycle operations and must not be
   conflated with buffer overflow.
6. Public input and output names stay on current canonical fields; no legacy
   aliases are introduced.
