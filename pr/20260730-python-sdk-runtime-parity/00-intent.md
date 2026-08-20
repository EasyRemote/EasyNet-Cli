# Intent

Close Python SDK runtime parity drift found by targeted descriptor/authority tests after the canonical runtime convergence gate passed.

Two root issues were found:

1. Python public receipt parsing already requires generic session authority fields (`issuer_ura`, `subject_ura`), but the internal Axon `SessionAuthorityBody` construction still used retired generated-field names (`backend_ura`, `user_ura`).
2. Python runtime ability tests still modeled bidi descriptor refs as `!bidi`, while the canonical Axon descriptor-ref admission actions are `invoke`, `read`, `manage`, `grant`, and `stream`. Bidi is an execution mode over the stream admission action, not a separate descriptor-ref action.

