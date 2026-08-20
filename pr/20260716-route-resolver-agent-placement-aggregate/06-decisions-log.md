# Decisions Log

## 2026-07-16

- Selected route hosted placement as part of the Agent aggregate read-projection slice because route resolution is invocation proof selection and must not own hosted identity file parsing.
- Chose fail-closed placement availability on aggregate load failure; route resolution may still use presence and directory projections, but cannot prove local hosted placement from missing aggregate state.
