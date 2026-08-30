# Decisions and evidence

## Decision

Add `media_pipeline_support_visible` to the Browser/Tauri lifecycle evidence
contract, ordered after `media_presented` and before input/control.

## Evidence target

- verifier validates the new step and its visible label;
- self-test emits the new step;
- frontend product-flow checker rejects verifiers that omit it;
- product readiness audit records that live Browser/Tauri evidence must include
  media pipeline support visibility.
