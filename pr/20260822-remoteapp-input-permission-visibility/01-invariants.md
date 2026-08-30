# Invariants

1. Permission descriptors and schema descriptions must mention input injection
   when the ability requests input permission.
2. Frontend boundary checks must require parsing `input_permission` from
   `remote_desktop.request_permission`.
3. Product-flow checks must require an executable `Request permission` CTA for
   `input_injection_unavailable` sessions.
4. The readiness audit must keep the input row incomplete until real OS input
   injection evidence exists.
