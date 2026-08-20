# Unary Cancel Signed Command Gate

## Objective

Close the executable architecture gap for A68. Unary cancellation must not
reuse the target invocation nonce or signature. It is its own
descriptor-bound `invocation.cancel` command, signed independently and bound to
the target lifecycle by canonical lifecycle hash.

## Invariants

1. `InvocationCancelCommand` carries `target_lifecycle_hash` and rejects
   unknown fields.
2. `SignedInvocation::prepare_cancel_command` builds a fresh invocation draft
   for `invocation.cancel`.
3. The cancel command binds the target by `prepared.canonical_hash_hex()`.
4. The cancel draft is prepared with an `invocation.cancel.caller` signer
   policy.
5. `request_cancel_signed` signs the prepared cancel command with the canonical
   signer before transport submission.
6. `request_cancel_signed` must not submit the original signed target
   invocation as the cancel request.
7. Tests must prove cancel command nonce independence and replay rejection.

## Effect

This slice does not change public behavior. It turns the current independent
signed cancellation design into a regression gate so future edits cannot
reintroduce nonce-consuming replay semantics.
