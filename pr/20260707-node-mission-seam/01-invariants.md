# Invariants

1. Mission is a generic daemon SDK profile, not an EasyRemote Pipeline runtime.
2. Node preserves complete Invocation carrier context for daemon-dispatched
   operations: `caller_ura`, `callee_ura`, `subject_ura`,
   `descriptor_version`, `nonce_base64`, and `causal_context`.
3. Mission IDs are opaque run identifiers and must not be path-like.
4. Run-file requests carry a path as an input fact; Node does not read the file
   or own filesystem policy.
5. MissionStatus child receipts are accepted only as daemon-projected facts;
   Node does not fabricate or verify them.
6. Mission event pages use explicit cursor and terminal state fields.
7. No non-URA naming and no legacy input aliases are introduced.
