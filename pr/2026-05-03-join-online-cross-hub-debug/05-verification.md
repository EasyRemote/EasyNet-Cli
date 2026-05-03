# Verification

## Targeted tests

- `cargo test --lib --features axon-pb parse_node_uri -- --nocapture`
  Result: pass.
- `cargo test --lib --features axon-pb auto_wire_writes_creds_hub_endpoint_not_peer_hub_guess -- --nocapture`
  Result: pass.
- `cargo test --lib --features axon-pb handle_list_user_devices -- --nocapture`
  Result: pass.
- `cargo test --lib --features axon-pb canonicalize_presence_key -- --nocapture`
  Result: pass.
- `cargo test --lib --features axon-pb forward_invoke_cross_tenant_with_peer_entry_dials_via_federation_client -- --nocapture`
  Result: initially failed because the test still asserted legacy `/agent/<bare-node>` target URIs; passes after aligning the test baseline to canonical `/device/<node>`.
- `cargo test --lib --features axon-pb forward_invoke_ -- --nocapture`
  Result: pass (25 tests).
- `cargo test --lib --features axon-pb dispatch_session_request_ -- --nocapture`
  Result: pass (7 tests).
