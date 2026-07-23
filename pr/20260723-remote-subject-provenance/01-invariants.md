# Invariants

- Every remote invocation tuple has an inspectable subject before transport.
- Public ingress can only use a caller-declared subject.
- Daemon-owned remote system/root issuers can only use a daemon-selected
  target-owned subject.
- No public subject omission, callee substitution, descriptor substitution, or
  fallback subject policy is represented in the remote invocation subject model.
