# Intent

## Goal

Close the Java SDK receipt proof-fact bypass so Java participates in the same canonical runtime receipt verification model as Go and Python.

## Non-goals

- Do not add product-specific EasyNet or EasyRemote receipt concepts.
- Do not change public Java SDK method names unless the existing shape cannot enforce mandatory proof facts.
- Do not preserve opaque or legacy proof profiles.

## Acceptance criteria

- Java receipt canonicalization rejects every required proof-fact omission with `RECEIPT_PROOF_FACTS_MISSING`.
- Java accepts only `axon-strict-v2` proof facts.
- Java tests cover every mandatory proof-fact omission.
- Architecture and SPEC v2 gates enforce the Java proof-fact guard.
