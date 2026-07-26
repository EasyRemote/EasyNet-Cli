Invariants:

1. Device/Both control discovery cannot become Ready unless invocation boot has
   produced `paired_user_runtime_signer`.
2. The capability flag is not cosmetic; it is a state-machine proof emitted by
   `register_paired_user_runtime_signer`.
3. `ready_runtime_discovery` must fail closed if Device/Both mode is missing the
   paired signer proof.
4. Hub mode remains independent from paired device credentials and user custody.
5. No all-zero or default principal may be introduced to satisfy readiness.
