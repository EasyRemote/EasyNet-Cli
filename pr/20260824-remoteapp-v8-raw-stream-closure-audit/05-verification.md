# Verification

Planned checks:

- exact ABI v7 and additive ABI v8 header/export gates plus mutation tests;
- focused Rust FFI raw stream tests;
- Python C ABI and stream tests;
- Go C ABI and stream tests;
- canonical Runtime convergence and SDK parity gates relevant to raw streams;
- direct inspection or executable smoke of the RemoteApp/EasyRemote consumer.

Passing ABI/SDK tests alone will not advance RemoteApp product readiness when
no product consumer exercises the raw path.

Completed so far:

- v8 static contract and new mutation/attack test pass;
- release package contract and mutation test pass;
- SDK product-neutrality and parity self-test pass;
- focused Python v8 selection/fallback tests pass;
- full Go runtime-cabi suite passed;
- 61 focused Python C ABI/stream tests passed;
- focused Rust v8 raw-delivery and receipt-error tests passed;
- the built `libeasynet_cli.dylib` passed exact v7/v8 export checks and Python
  feature discovery selected v8 from the actual library;
- EasyRemote typed-media/C ABI stream facade tests passed;
- the opt-in EasyRemote live v8 smoke passed against the current local daemon
  and `target/debug/deps/libeasynet_cli.dylib`: three typed raw frames (including
  arbitrary 16 KiB bytes, embedded NUL bytes, and an empty payload) arrived
  byte-for-byte with their content type preserved;
- RemoteApp product closure static gate and mutation/attack suite passed;
- forwarded remote server-stream projection now has focused coverage proving
  `video/h264` remains coupled to its raw payload instead of being rewritten as
  the federation JSON default;
- the Python C ABI adapter now preserves observed Runtime sequence values
  instead of repairing duplicate/regressing values; the shared `StreamHandle`
  rejects duplicates fail-closed, and the ABI mutation suite pins both facts;
- the direct v8 C callback metadata contract now has normative JSON wire types;
  Rust converts the protobuf state enum to its canonical Runtime state name
  before v8 delivery, with a focused exact-type regression test;
- `cargo test --locked --features remote-desktop
  forwarded_remote_stream_chunk_preserves_typed_payload_content_type --lib`
  — PASS;
- `cargo test --locked --features remote-desktop
  invoke_stream_dispatches_remote_selected_route_over_presence_session --lib`
  — PASS;
- the real Frontend Browser runner passed one complete local window lifecycle:
  authentication, target selection, permission preflight, consent, production
  create-session/set-description/watch-events, connected WebRTC, audio/video
  tracks, four rendered frames, explicit Accessibility policy block, normal
  end-session, and a visible caller-ended terminal receipt;
- the live Docker combined smoke passed 58/58 governed cross-device routing
  assertions and 45/45 synthetic stream/bidi assertions across distinct Device
  URAs; the report preserves dirty-source and prebuilt-image provenance and is
  intentionally not clean-build completion evidence;
- the synthetic media fixture is on descriptor schema v3, and receipt tuple
  validation now binds callee to the catalog owner Agent rather than confusing
  it with the Device execution host;
- the aggregate SDK cutover self-test reached an unrelated pre-existing sibling
  evidence failure: EasyNet-Axon lifecycle `execution-report.v1.json` has a
  stale source digest. No sibling evidence was rewritten.

The live smoke proves the v8 server-stream consumer path. It does not replace
RemoteApp's WebRTC media plane and is not evidence that interactive desktop is
product-complete.

Remaining evidence seam: the temporary EasyRemote live smoke proves byte-exact
typed delivery and observes v8 feature availability, but it does not expose an
independent carrier-selection diagnostic proving that the live call invoked the
v8 symbol rather than a byte-equivalent v7 fallback. A checked-in consumer
runner/report with carrier-selection evidence is still required for a fully
reproducible v8 consumer closure claim.
