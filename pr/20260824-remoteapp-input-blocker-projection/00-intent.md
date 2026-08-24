# Intent

RemoteApp interactive sessions currently collapse every target lifecycle input
block into `target_input_not_ready`. A real macOS browser lifecycle proved that
the selected TextEdit window emitted `TARGET_BLURRED`, while the host also
reported Accessibility input permission as unavailable. The coarse projection
hid both actionable causes and suggested `retry_session`, which cannot restore
host focus.

This slice preserves the fail-closed target-local input guard while projecting
the authoritative target and platform blocker. Focus loss now asks the operator
to focus the selected target on the host; it does not claim that recreating the
session repairs OS focus.
