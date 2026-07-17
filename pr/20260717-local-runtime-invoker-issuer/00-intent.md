# Local Runtime Invoker Issuer Cutover

Close the RF-8 daemon-local tuple construction fork in
`local_runtime_invoker.rs`.

`InvocationTarget` already exposes subject and causal context as explicit
policy states. The remaining defect is that the LocalRuntime adapter still
materializes `_system.local` caller identity, nonce, subject identity, and
descriptor-bound envelope parts directly. This slice keeps target policy in the
daemon dispatch layer but moves canonical system request construction to
`SystemInvocationIssuer`.
