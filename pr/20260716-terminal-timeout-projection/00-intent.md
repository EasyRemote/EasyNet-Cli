# Intent

The daemon receipt projection collapsed Axon's `TimedOut` terminal state into
`Failed`. This discarded an already-verified lifecycle fact and made schedule
and loop consumers unable to distinguish timeout policy from handler failure.

This slice makes the daemon presentation state machine an exact projection of
the Axon terminal vocabulary, while preserving the existing failure reason
shape for both failure and timeout outcomes.
