Verification for Python profile error source refs:

- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_errors.py sdk/python/tests/test_publication.py sdk/python/tests/test_easyremote_profiles.py sdk/python/tests/test_receipt.py -q`
- `PYTHONPATH=sdk/python python -m ruff check sdk/python/easynet_sdk/errors.py sdk/python/easynet_sdk/directory.py sdk/python/easynet_sdk/identity.py sdk/python/easynet_sdk/publication.py sdk/python/easynet_sdk/mission.py sdk/python/easynet_sdk/host_binding.py sdk/python/easynet_sdk/events.py sdk/python/easynet_sdk/surface.py sdk/python/easynet_sdk/compatibility.py sdk/python/easynet_sdk/wrappers.py sdk/python/easynet_sdk/admin.py sdk/python/easynet_sdk/easyremote_profiles.py sdk/python/easynet_sdk/receipt.py sdk/python/tests/test_errors.py sdk/python/tests/test_publication.py sdk/python/tests/test_easyremote_profiles.py sdk/python/tests/test_receipt.py`
- `PYTHONPATH=sdk/python python -m pytest sdk/python/tests -q`
- `bash tools/scripts/check-sdk-scaffold.sh`
- `PYTHONPATH=sdk/python python -m ruff check sdk/python`
