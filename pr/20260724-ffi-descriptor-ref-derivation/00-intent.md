# FFI descriptor-ref derivation convergence

## Goal

Remove duplicate descriptor-ref derivation from the FFI runtime descriptor catalog and route every descriptor-ref projection through `AbilityDescriptor::descriptor_ref()`.

## Non-goals

- Do not change descriptor wire shape.
- Do not change route resolution or descriptor miss behavior.
- Do not add a compatibility fallback to `meta.list_abilities` or remote probing.

## Acceptance criteria

- FFI catalog projection uses the canonical descriptor method.
- FFI no longer formats `<ability>@<version>#<hash>!<action>` itself.
- Convergence gate rejects reintroduced FFI-local descriptor-ref formatting.
- Existing descriptor resolver tests and convergence gates pass.
