# Invariants

1. Events is a generic daemon SDK profile; it must not contain EasyRemote,
   backend route, browser, or GUI product semantics.
2. Node Events carriers must preserve the complete Invocation tuple context:
   `caller_ura`, `callee_ura`, `subject_ura`, `descriptor_version`,
   `nonce_base64`, `causal_context`, and ability-specific args.
3. Node may validate and serialize carriers, but daemon/Axon providers own
   carrier projection, stream filtering, event ordering, and terminal facts.
4. Directory/device/invocation/session event streams must use explicit stream
   kinds and explicit cursors; no facade-side fan-out or post-filtering.
5. Session streams require `session_id`; product `session_ura` parsing is not
   accepted as a compatibility path.
6. Event stream lifecycle must reuse the existing bounded `StreamHandle` seam.
7. No non-URA naming and no legacy input aliases are introduced.
