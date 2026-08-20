# Decisions Log

## 2026-07-23

- Decision: refresh the canonical fixture instead of weakening production gates.
- Reason: the real checkout already passed the production gate; the failing
  surface was the shell self-test's outdated accepted model.
- Decision: keep negative fixture coverage and update only the retired R95 rule
  marker.
- Reason: the FFI descriptor remote-probe rule was renamed/converged into the
  bounded catalog gate. The forbidden behavior remains covered.
