# Verification

## SDK parity

```text
(cd sdk/go && go test ./...)
ok   easynet.run/cli/sdk/go
ok   easynet.run/cli/sdk/go/runtimeevents

(cd sdk/python && .venv/bin/python -m pytest -q tests/test_runtime.py tests/test_signing.py)
44 passed, 3 subtests passed
```

The Go and Python runtime transport fixtures remove `prepared_id` when
`material_only=true`. Each facade extracts `SigningMaterial` successfully,
while its public retained-prepared decoder still rejects that same payload.

## Native and browser boundary

```text
cargo test --lib invocation_prepare_material_only_does_not_allocate_a_prepared_handle
finished successfully

go test -tags 'easynet_cabi backend_live_daemon' ./internal/handler \
  -run '^TestBridgeHTTP_E2E_RegisteredBrowserInvokesHubAbilityThroughLiveDaemon$' \
  -count=1 -v
PASS
```

The live browser HTTP flow registers a browser signing key, prepares canonical
material through the SDK, signs it outside the daemon, and submits a successful
Hub-owned `meta.list_abilities` invocation. No browser response carries a
process-local native prepared capability.
