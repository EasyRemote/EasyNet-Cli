# Invariants

1. Python direct runtime must not parse DescriptorRef or URA grammar.
2. Unary, stream, and bidi direct runtime paths use one shared identity-backed
   ability projection path.
3. Missing identity projection capability fails closed before any daemon gRPC
   invocation is attempted.
4. The emitted Axon target/function fields use the public ability name from
   identity-projected ability address facts.
5. The callee in the Invocation draft must own the projected ability URA.
6. Connector-owned identity facades are closed exactly once.
7. Go and Python direct runtime projection semantics remain architecturally
   equivalent.
