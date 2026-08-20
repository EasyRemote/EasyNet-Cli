# Clean Runtime Bootstrap Repro Plan

## Goal

Verify and fix the clean-environment EasyNet daemon bootstrap path after old local data is removed. The target is a self-consistent product runtime where caller signer custody, device descriptor advertisement, and authority subject binding are created by daemon bootstrap instead of inherited from stale state.

## Invariants

- The daemon must not depend on legacy `~/.easynet` records to make a new user/device callable.
- A public invocation must enter with explicit caller, callee, ability, subject, nonce, causal context, and args.
- Caller signing material must be provisioned through the daemon key service; SDK/provider code must not generate fallback keys.
- Device ability descriptors must be advertised before product UI/backend attempts `meta.list_abilities`, `meta.list_resources`, or `invocation.history.list`.
- Authority session subject must admit the envelope subject. Zero UUID or unrelated user-scoped sessions must not authorize device-scoped invocations.
- Route failures must distinguish offline owner, missing descriptor, admission denial, and signer-custody failure.

## Boundary Proof

- Axon owns descriptor-bound invocation, admission, receipt facts, and terminality.
- EasyNet-Cli daemon owns product bootstrap: local data directory, key-service custody, device/user projection, ability registration, and authority binding.
- Product UI/backend may request ability state, but must not become a descriptor, signing, or receipt authority.
- Clearing old data is permitted for this work, so no compatibility layer will be added for stale device IDs, orphaned descriptors, or missing keyring entries.

## Work Plan

1. Snapshot current tree and preserve user documentation changes.
2. Stop any running daemon and remove EasyNet local runtime state.
3. Rebuild and start the daemon in a clean environment.
4. Reproduce `invocation.history.list`, `meta.list_abilities`, and `meta.list_resources` from product-visible entry points.
5. Fix the bootstrap/root authority path if clean start does not provision signer, descriptor, or authority subject correctly.
6. Run SPEC v2, cutover readiness slices, Docker media/bidi e2e, and formatting checks.

## Decisions

- No legacy-state compatibility will be implemented. If old local records are incompatible, they are deleted for this verification path.
- Errors caused by missing current signer or descriptor advertisement are treated as daemon bootstrap bugs, not SDK facade bugs.
