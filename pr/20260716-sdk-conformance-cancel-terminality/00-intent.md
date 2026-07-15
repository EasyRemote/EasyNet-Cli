## Intent

Close the SDK conformance gate cancellation fork. The wrapper already owns
language sequencing and uses `run_bounded` for child processes, but an
interrupted child returned `130` as an ordinary language failure, allowing the
top-level loop to continue validating later languages.

This slice makes cancellation a single terminal gate state. It does not change
SDK runtime behavior, public SDK APIs, adapter report schemas, or conformance
case semantics.
