# Decisions Log

- Decision: remove `Deserialize` instead of adding `deny_unknown_fields`.
  - Reason: this is a daemon-produced read model, not an input schema. Output-only ownership is clearer and eliminates the accidental compatibility surface entirely.
- Decision: extend the output-only rule to `surface.rs` and `broker.rs`.
  - Reason: these types contain realtime activation plans and are also daemon-generated product reports.
- Decision: leave plugin manifest and sidecar schemas unchanged.
  - Reason: those are the actual input contracts and already carry strict unknown-field rejection.
