# Admission signature reason canonicalization

## Goal

Remove legacy signature-denial alias parsing from admission diagnostics. Signature
reasons must be emitted and projected as canonical RFC-014 reason facts instead
of substring-scanned compatibility details.

## Root abstraction problem

Admission currently emits `CALLER_UNKNOWN` for local principal/key misses, while
diagnostic projection maps that legacy alias back into `CALLER_KEY_NOT_FOUND`
with broad `contains()` matching. That creates two authorities for the same
semantic denial: the producer emits one vocabulary and the projection repairs it.

## Invariants

1. Missing caller key/principal material is reported as canonical
   `CALLER_KEY_NOT_FOUND`.
2. Signature diagnostic projection recognizes canonical reason tokens at the
   status-message boundary only.
3. Detail text must not be scanned for aliases or unrelated words.
4. Unknown/opaque signature failures remain fail-closed as
   `CALLER_SIGNATURE_VERIFY_FAILED`.
5. No public invocation tuple fields or SDK public APIs change.

## Boundary proof

The change belongs in daemon admission, not in product UI or SDK facades. The
producer owns canonical reason emission and the RFC-014 decision DTO owns
projection from transport status into diagnostic facts. Removing alias repair at
this boundary reduces coupling because downstream explain/history surfaces no
longer need to understand historical daemon vocabulary.

## Verification plan

- Rust admission decision unit tests.
- Rust admission facade unit tests covering unknown caller status emission.
- SPEC v2 gate and self-test.
- Architecture convergence gate.
- `cargo fmt --check`.
- `git diff --check`.
