# Intent

Prove `admission.explain` projects voice-call actions from persisted signed
descriptor facts.

Voice call abilities mix RPC and stream actions. The explain surface must report
the action bound into the invocation ledger record and descriptor reference, not
infer action from the current live descriptor catalogue or from voice-specific
ability names.
