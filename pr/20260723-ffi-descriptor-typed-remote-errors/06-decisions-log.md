# Decisions Log

## 2026-07-23

- Decision: remove FFI descriptor error classification based on remote daemon
  message substrings.
- Reason: SDK-facing canonical runtime state must be selected by typed runtime
  boundaries, not product/runtime wording that can change independently.
- Decision: introduce `RemoteInvocationFailure` in the remote invocation adapter
  and route the descriptor probe through `invoke_remote_target_with_caller_signer_typed`.
- Reason: the remote invocation boundary is the correct owner for transport,
  daemon-status, terminal-state, and result-decode failure states.
- Decision: remove the now-unused anyhow adapter that accepted a preloaded
  caller signer.
- Reason: retaining it would be a compatibility layer with no production caller.
