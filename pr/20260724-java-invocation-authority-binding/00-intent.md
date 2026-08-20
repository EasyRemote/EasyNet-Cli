# Intent

Close the Java SDK authority-binding divergence.

Java `InvocationBuilder.inspect()` currently validates only authority metadata envelope shape. Node, Go, and Python validate the typed authority metadata against the descriptor-bound invocation tuple before transport. Java must converge to the same canonical runtime model so stale caller/callee/subject authority does not pass the SDK facade and fail later in daemon admission.

