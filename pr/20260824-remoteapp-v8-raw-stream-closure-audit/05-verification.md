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
- RemoteApp product closure static gate and mutation/attack suite passed;
- the aggregate SDK cutover self-test reached an unrelated pre-existing sibling
  evidence failure: EasyNet-Axon lifecycle `execution-report.v1.json` has a
  stale source digest. No sibling evidence was rewritten.
