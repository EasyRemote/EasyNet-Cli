# Decisions and Deltas — RemoteApp Product Closure

## 2026-08-22 — User Service projection conflict must not kill Device session

Decision:

- `service/<user>.pages` remains a user-scoped Service owner projection.
- The Hub read model currently selects one live projection row per
  `owner_ura`.
- Equal generation/revision projection conflicts are read-model selection
  outcomes, not authority failures.
- Device-native RemoteApp abilities remain SystemAgent-owned and must not be
  taken offline by a non-selected user Service projection.

Implementation delta:

- `federation.advertise_abilities` responses now carry an optional projection
  upsert `outcome`.
- Strict projection callers still require `ack=true` and exact `count`.
- User-scoped Service owner prelude degrades only when the admitted write is a
  read-model rejection such as `ignored_stale` or `rejected_conflict`.
- Admission, signer delegation, descriptor integrity, transport errors, and
  acknowledged count mismatches still fail closed.

Product effect:

- Cross-device RemoteApp smoke should no longer report the caller Device as
  offline merely because another host already owns the selected Pages Service
  projection.
- This does not claim product completion for real OS capture, input injection,
  audio/video, NAT/relay, or frontend end-to-end RemoteApp lifecycle.
