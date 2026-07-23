# Caller signer readiness error sanitization

## Goal

Remove product-visible leakage of keyring implementation details from remote
caller signer readiness failures. Missing caller signer custody should surface as
a typed runtime readiness failure, not as raw `keyring entry not found` text.

## Invariants

1. Remote invocation must still fail before daemon socket probing when caller
   signer custody is missing.
2. The public error must name the caller signer readiness stage and caller URA.
3. The public error must not expose keyring implementation vocabulary or storage
   details.
4. Federation-local signer loading and generic remote invocation signer loading
   must share the same sanitized readiness projection.

## Boundary proof

- `src/daemon/invocation/routing/remote_invoke.rs` is the daemon routing boundary
  that turns signer custody failures into product-visible CLI/SDK errors.
- It must not preserve `{err}` from `self_identity`/keyring in the public message.
- The SPEC v2 gate should reject remote signer readiness errors that interpolate
  raw lower-layer errors into `requires a caller signer` messages.

## Verification plan

1. Run targeted `remote_invoke` Rust tests.
2. Run SPEC v2 convergence gate.
3. Run architecture convergence gate.
4. Run `cargo fmt --check` and `git diff --check`.

## Decision log

- Treat missing signer custody as a typed runtime readiness condition. The
  keyring remains the provider of signer custody, but its storage miss text is
  not part of the public runtime contract.
