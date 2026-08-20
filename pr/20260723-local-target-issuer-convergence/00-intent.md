# Intent

Collapse daemon-system target-derived subject issuance onto
`SystemInvocationTargetIssuer::local_root_for_target`.

`LocalDaemonSystemAbilityIssuer::invoke_target_root_derived_subject_timeout`
preserved a second convenience path that derived a subject from
`LocalAbilityTarget` before crossing into local invoke transport. The routing
target issuer already owns that policy. This slice migrates callers and removes
the duplicate helper.
