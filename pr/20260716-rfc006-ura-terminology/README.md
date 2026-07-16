# RFC-006 URA Terminology Convergence

## Goal

Remove the remaining EasyNet identity/address `URI` terminology from the
normative RFC-006 state-object document. The architecture naming rule is URA
only; URI may remain only for actual HTTP/Hyper transport types or historical
non-normative records.

## Root Fork

`docs/AXON-RFC-006-stateful-easynet.tex` is one of the normative files used to
decide EasyNet runtime boundaries. It still described owner agents and
principal callers as `URI`, and exposed a `caller-uri` section label. That made
the normative source disagree with the current URA-only runtime model.

## Decision

Update RFC-006 identity terminology to URA:

- owner-agent identity is an EasyNet Agent URA;
- principal caller scheme is a URA scheme;
- principal caller parser work is named URA parser/classifier work;
- internal labels and references move from `caller-uri` to `caller-ura`.

## Boundary Proof

- This is not a transport URI rename. HTTP/Hyper `Uri` types remain outside the
  scan because they describe transport endpoints, not EasyNet routable
  identities.
- No public runtime behavior changes.
- The convergence gate now rejects identity/address `URI` wording in the
  normative RFC-006 document.

## Verification Plan

- `rg -n "\bURI\b|\bURIs\b|caller-uri|principal-URI" docs/AXON-RFC-006-stateful-easynet.tex`
- `bash tests/scripts/test_check_architecture_convergence.sh`
- `bash tools/scripts/check-architecture-convergence.sh`
- `git diff --check`
