# Architecture

Layering:

1. Host lifecycle verifiers validate raw public ability evidence.
2. Host lifecycle verifier reports publish compact summaries of the verified lifecycle fact.
3. Product-completion gate validates those summaries across the required window/application matrix.

The aggregate gate does not own CLI invocation, daemon state transitions, or receipt construction. It only rejects weak reports that do not summarize the required terminal/non-terminal lifecycle proof.
