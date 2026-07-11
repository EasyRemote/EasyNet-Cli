# Verification

```text
go test -count=1 ./sdk/go
python -m pytest sdk/python/tests/test_runtime_environment.py
python -m pytest tests/test_config.py tests/test_identity.py
bash tools/scripts/check-downstream-sdk-consumer-cutover.sh --self-test
bash tools/scripts/check-downstream-sdk-consumer-cutover.sh /Users/macbook.silan.tech/Documents/GitHub/EasyNet/backend /Users/macbook.silan.tech/Documents/GitHub/EasyRemote
bash tools/scripts/check-sdk-parity-matrix.sh
git diff --check
```
