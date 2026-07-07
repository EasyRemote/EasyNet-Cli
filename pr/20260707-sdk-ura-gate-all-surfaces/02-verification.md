# Verification

Status: Passed.

Commands run:

```sh
bash tools/scripts/check-sdk-ura-naming.sh
TMPDIR=/tmp bash tools/scripts/check-sdk-scaffold.sh
bash tools/scripts/check-sdk-completion-audit.sh
git diff --check
```

Notes:

- The widened gate initially walked into `sdk/python/.venv`, which is
  third-party dependency content. The final guard prunes dependency/build
  directories while scanning authored SDK docs, tests, language facades,
  schemas, and conformance artifacts.
