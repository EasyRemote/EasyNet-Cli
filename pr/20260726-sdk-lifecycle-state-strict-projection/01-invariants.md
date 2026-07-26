# Invariants

## Semantic invariants

- Runtime receipt `state` is a canonical runtime lifecycle fact.
- `receipt_type` remains the lowercase receipt-class discriminator.
- SDK language bindings may expose language-native enum names internally, but the accepted receipt carrier value must be canonical.
- Terminality must be derived from the canonical lifecycle model, not from string normalization.

## Safety invariants

- Unknown or retired state spelling fails closed before receipt projection is considered valid.
- `Unspecified` remains invalid for runtime receipt summaries.
- Receipt proof facts remain mandatory and are not bypassed by lifecycle parsing.

## Boundedness invariants

- Parsing is finite and explicit.
- No regex normalization, case folding, trimming, or punctuation folding is used to accept lifecycle state.
- No compatibility fallback path is introduced.
