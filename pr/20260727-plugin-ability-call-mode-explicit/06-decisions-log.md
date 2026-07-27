## 2026-07-27

- Decision: Make plugin ability `call_mode` mandatory.
- Reason: Invocation call mode affects descriptor hash, route selection, and stream/bidi terminality; parser defaults make product plugin behavior under-specified.
- Scope: Plugin manifest parser and repository fixtures only.
- Verification boundary: accept focused plugin parser/package/install/registration tests plus SPEC gates; full plugin module test currently includes unrelated credential-dependent remote-desktop attach cases in a clean local runtime.
