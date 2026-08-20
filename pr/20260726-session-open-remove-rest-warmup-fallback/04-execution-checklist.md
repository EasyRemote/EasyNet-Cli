# Execution Checklist

- [x] Identify REST credential warmup with codegraph.
- [x] Confirm it is only used by the session initiator.
- [x] Remove the warmup module/import/call.
- [x] Remove warmup-specific tests and helper imports.
- [x] Add convergence gate coverage against REST credential warmup.
- [x] Run targeted tests and SPEC gates.
