# Architecture

Layering:
- `resource_subject.rs` is the shared core resolver for media and media-adjacent plugin resource subject ingress.
- Media handlers consume typed `ResourceEntry` results and should not parse subject URAs directly.
- Remote desktop plugin permission/status paths may query whether a subject is resource-scoped, but that check must use the same classifier as strict resolution.

Refactoring direction:
- Replace implicit `bool` subject classification with a private state enum.
- Keep outward error strings stable while making internal state explicit and testable.
