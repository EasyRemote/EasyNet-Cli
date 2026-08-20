# Invariants

1. Invocation authority metadata is not valid merely because it is well-shaped.
2. Delegation authority must bind the invocation caller, subject, audience, and ability scope.
3. Session authority must bind the invocation caller, callee, subject admission predicate, audience, action, and ability scope.
4. Java SDK errors must use canonical authority codes for tuple-binding failures.
5. Public Java SDK API surface remains unchanged.
6. Java validation semantics converge with Node, Go, and Python rather than creating a Java-specific lifecycle.

