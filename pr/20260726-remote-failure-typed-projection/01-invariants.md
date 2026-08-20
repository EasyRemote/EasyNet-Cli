# Invariants

## Semantic invariants

- `SessionFailure` is the remote failure semantic authority.
- Raw transport error text is diagnostic evidence only when typed failure facts are missing.
- A missing terminal/admission receipt remains a failed-precondition boundary unless typed failure facts prove a more specific class.

## Safety invariants

- Authority denial must not be inferred from arbitrary raw text.
- Caller signer/keyring details must not leak when a typed signer-readiness failure is present.
- Owner-offline route status must remain distinguishable from ability absence when typed route facts are present.

## Boundedness invariants

- Raw error fallback has one deterministic status class.
- No substring classifier may promote raw text into a canonical security or route state.
- Failure projection remains side-effect free.
