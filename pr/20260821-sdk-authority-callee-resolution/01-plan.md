# SDK Authority Callee Resolution

## Problem

`AuthorizedRuntimeSession` kept treating `RuntimeTargetRef.URA` as the Invocation callee. When the target is a Device execution host, SDKs could construct authority metadata and Invocation drafts where the Device acted as the callable owner. That violates the target ontology:

```text
target/execution_host = Device
callee = Agent | Service | Authority that advertises the descriptor
```

The daemon admission layer already rejects Device authority audiences, but the Go and Python SDK facades could still mint or accept those malformed authorities before dispatch.

## Design

- Keep `RuntimeTargetRef` as the execution target selected by product/session UX.
- Add `resolved_callee_ura` to descriptor resolution results.
- Derive the resolved callee from the descriptor_ref owner when the provider does not fill it.
- Build prepared Invocation drafts with the resolved callee, not the target.
- Validate authority metadata so delegation audience, session callee, and session audience are callable owners: Agent, Service, or Authority.
- Keep Device URAs valid as subjects/resources where the descriptor contract allows acting on a device.

## Expected Effect

SDK session calls now preserve the clean tuple:

```text
caller = backend/user/agent caller
target = device execution host
callee = descriptor owner / callable SystemAgent or Service
subject = resource, state, session, or device being acted on
```

This prevents frontends and SDK callers from preparing the exact class of `AUTHORITY_REQUIRED` / subject-callee mismatch failures caused by Device-as-callee authority metadata.
