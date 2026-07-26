Goal
====

Converge governance-read route admission so receipt-history and runtime-catalogue
reads are admitted by one selected-route policy before either LocalRuntime or
presence dispatch.

Non-goals
=========

- Do not add a product-specific EasyNet/EasyRemote route.
- Do not preserve a remote-only compatibility gate.
- Do not change public ability names or descriptor refs.
- Do not implement new browser/media product abilities.

Acceptance criteria
===================

- Local selected-route dispatch rejects receipt-history public-action subjects
  before Axon admission can emit `AUTHORITY_SUBJECT_MISMATCH`.
- Remote selected-route dispatch keeps the same governance-read checks.
- Stream and bidi selected-route dispatch share the same policy naming and
  module ownership.
- SPEC v2 gate continues to enforce the selected-route policy and no duplicate
  history predicate is introduced.
