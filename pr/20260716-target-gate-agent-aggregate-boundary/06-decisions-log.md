# Decisions Log

## 2026-07-16

- Selected admission target locality as the next Agent aggregate root-fork slice because it directly gates unary, stream, and bidi local dispatch.
- Chose fail-closed Agent URA locality on aggregate load failure. Partial registry or identity evidence is not sufficient to prove local Agent ownership at admission.
- Moved hosted Agent target parsing into the aggregate owner so admission consumes a domain value object instead of carrying its own copy of Agent URA parsing.
