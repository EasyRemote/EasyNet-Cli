# Verification

Targeted checks:

```bash
PYTHONPATH=/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Cli/sdk/python:/Users/macbook.silan.tech/Documents/GitHub/EasyNet-Axon/sdk/python:$PYTHONPATH \
  python -m pytest sdk/python/tests/test_runtime.py sdk/python/tests/test_authorized_runtime_session.py sdk/python/tests/test_runtime_ability.py \
    -k 'session_authority or receipt_history or runtime_state or descriptor or authority_binding' -q

cargo test -q descriptor_ref
cargo test -q invocation_history

(cd sdk/go && go test ./... -run 'SessionAuthority|ReceiptHistory|RuntimeState|Descriptor|AuthorizedRuntimeSession' -count=1)
```

