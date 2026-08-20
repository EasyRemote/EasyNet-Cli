# Intent

## Goal

Converge the SDK authority and descriptor-resolution paths on the target ontology:

```text
Device = execution substrate
callee = callable descriptor owner: Agent | Service | Authority
AbilityDescriptor != AbilityImpl
Invocation = caller, callee, ability, subject, nonce, causal_context, args
```

## Non-goals

- Do not rename the public `RuntimeDescriptorRefRequest.callee_ura` wire field in this change; that is a public API/SPEC migration.
- Do not merge the experimental `refresh_remote_targets` operator exposure worktree without a separate authority and frontend proof.
- Do not change RemoteApp session architecture in this slice; only audit that its separate branch is not mixed into this checkout.

## Acceptance criteria

- Go and Python SDKs prepare AuthorizedRuntimeSession drafts with descriptor-owner callees, not Device targets.
- Concrete Device URAs are rejected as session authority callees and concrete authority audiences.
- Selector audiences (`*`, realm prefix) remain valid where existing delegation semantics require them.
- Python descriptor-resolution provider paths project Device catalogue targets to the device-sponsored runtime-introspection SystemAgent like Go.
- Downstream EasyNet backend tests mint authority fixtures against callable owners.
- SDK public API/conformance evidence is current.
- Cross-repo SDK cutover readiness passes.
