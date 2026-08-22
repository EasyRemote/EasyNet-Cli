# Invariants

1. Product status remains `incomplete` and `product_complete=false`.
2. Current evidence may strengthen the local macOS bounded product-flow row,
   but must not claim Windows/Linux capture, host audio, real input injection,
   NAT/relay fallback, Browser/Tauri lifecycle, or cross-device product
   completion.
3. Evidence must name the report artifact and the executed target kind
   (`both`) so later audits can distinguish current local evidence from stale
   or partial window-only reports.
4. The audit must preserve Device/SystemAgent/plugin boundaries: RemoteApp
   remains a daemon plugin AbilityImpl behind governed `remote_desktop.*`
   Ability URAs.
