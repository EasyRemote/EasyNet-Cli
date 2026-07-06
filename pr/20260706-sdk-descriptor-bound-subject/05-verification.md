# Verification

Executed commands:

```sh
cd sdk/go && go test ./...
cd sdk/go && go test . -run TestPublicGoSDKDoesNotImportForbiddenRuntimeBoundaries
PYTHONPATH=sdk/python python -m pytest sdk/python/tests/test_identity.py
python -m py_compile sdk/python/easynet_sdk/identity.py sdk/python/easynet_sdk/__init__.py sdk/python/tests/test_identity.py
git diff --check
! rg -n 'func DescriptorBoundResourceSubjectURA|sdkURASchemeRoot|fmt\.Sprintf\(.*easynet:///r|realm \+ descriptor owner id|Do not add a Go' sdk/go/subject.go sdk/go/subject_test.go pr/20260706-sdk-descriptor-bound-subject --glob '!05-verification.md' -S
```

Results:

- Go SDK tests passed.
- Python identity tests passed.
- Python compile check passed.
- `git diff --check` passed.
- Go SDK import-boundary test passed.
- Go MEMC conformance now assigns `IdentityClient.ResourceURA` and
  `IdentityClient.DescriptorBoundResourceSubjectURA` to `directory_identity`.
- Descriptor-bound subject search finds no package-level Go helper, SDK URA
  scheme constant, or subject helper string-construction residue in this slice.
- A broader SDK scan still finds pre-existing static URA construction in
  `sdk/go/surface_runtime.go`; that belongs to a separate surface runtime
  convergence slice.
