# Architecture

The descriptor resolver is an FFI-facing adapter around canonical runtime
descriptor lookup. It is allowed to probe a remote owner only by constructing a
complete remote Invocation through the canonical remote issuer.

Subject ownership is not a string utility. It is runtime state derived from the
callee owner kind. A small enum keeps that decision explicit and makes future
owner kinds impossible to add accidentally.
