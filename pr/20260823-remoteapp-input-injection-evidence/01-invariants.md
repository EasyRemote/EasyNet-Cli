# RemoteApp input injection evidence invariants

## Evidence invariants

- `proof_mode` must be `real_input_injection_matrix`.
- `component_mock` must be false.
- `real_backend_runtime` must be true.
- `product_complete_claim` must be false.
- The artifact must include platform entries for `macos`, `windows`, and
  `linux`.

## Passing scenario invariants

For a passing platform scenario:

- `selected_resource_ura` must be canonical.
- `session_id` must be present.
- `permission.accessibility_granted` or equivalent platform input permission
  must be true.
- `consent_scope` must be `input_control`.
- `input_scope` must be `display_global`.
- `focus_validated` must be true.
- `coordinate_mapping_validated` must be true.
- `target_geometry_revision` must be a positive integer.
- pointer and keyboard results must be `input_applied`.
- applied events must preserve `client_sequence` and `client_sent_at_ms`.
- host applied latency must stay within the artifact thresholds.
- `terminal_receipt` must be visible and bind the same session id.

## Unsupported scenario invariants

- Unsupported state is allowed only for Windows/Linux.
- Unsupported state must be explicit: `status=unsupported`,
  `unsupported_state=explicit_product_unsupported`, and `show_unsupported=true`.
- Unsupported scenarios must not report applied pointer/key input.

## Product boundary

This verifier defines the input injection artifact required before product
completion. It does not create real OS input evidence by itself.
