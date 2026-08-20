# Execution Checklist

- [x] Reproduce the LocalRuntime pages publish failure.
- [x] Identify duplicate fixture policy derivation with codegraph/search.
- [x] Delete the local fixture policy hash helper.
- [x] Replace it with `daemon::identity::signer_policy_ref`.
- [x] Add/extend a convergence gate against retired fixture policy namespace.
- [x] Run targeted tests and SPEC gates.
