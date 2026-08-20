# Verification

## Passed

- `if rg -n 'AXON-RFC-006-B-easynet-webapp\\.v|\\.tex\\.bak|backup file is canonical|Backups:' docs/rfc/AXON-RFC-006-B-easynet-webapp.tex; then exit 1; else echo 'rfc006-b backup authority: OK'; fi`
  - Result: `rfc006-b backup authority: OK`
- `if rg -n '\\bURI\\b|\\bCapability-URI\\b|_uri\\b|\\buri\\b' docs/rfc/AXON-RFC-006-C-openai-compat.tex; then exit 1; else echo 'rfc006-c URI naming: OK'; fi`
  - Result: `rfc006-c URI naming: OK`
- `bash tools/scripts/check-architecture-convergence.sh`
  - Result: `architecture-convergence: OK`
- `bash tools/scripts/check-project-structure-v1.sh`
  - Result: `project-structure-v1 ok` after removing generated
    `sdk/conformance/__pycache__`

## Notes

The deleted RFC-006-B `.bak` snapshots are historical drafts, not public
runtime behavior or compatibility files. Active v0.6 text now carries the
canonical source statement.
