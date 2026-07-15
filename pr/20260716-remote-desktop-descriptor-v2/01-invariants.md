# Invariants

- Every bundled `remote_desktop.*` ability descriptor uses schema version `2`.
- Every descriptor carries `descriptor_version`, `call_mode`,
  `capability_state`, `admission_action`, visibility scope, receipt schema,
  hints and `receipt_semantics`.
- `remote_desktop.attach` is explicitly `bidi`; `remote_desktop.watch_events`
  is explicitly `stream`; the remaining remote-desktop abilities are `rpc`.
- Read-only status/show abilities use read admission; state-changing control
  abilities use manage admission; media/event streaming uses stream admission.
- Descriptor metadata stays in plugin descriptors, not in hard-coded runtime
  fallback tables.
