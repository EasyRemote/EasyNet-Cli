# FFI descriptor authority tuple closure

## Scope

Close the remaining descriptor-resolution seam where remote catalogue
SessionAuthority validation silently projected missing request tuple fields as
empty strings before rejecting the authority.

## Invariants

- Public descriptor-resolution authority checks must not synthesize missing
  `callee_ura` or `subject_ura`.
- A remote catalogue SessionAuthority must bind the explicit request
  caller/callee/subject tuple.
- Failure remains fail-closed and typed as runtime attachment unavailable.
- No successful descriptor-resolution path changes.

## Verification

- `cargo fmt --check`
- `git diff --check`
- focused FFI descriptor-resolution tests
- RemoteApp boundary gates remain green
