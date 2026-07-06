# Architecture

Runtime Core remains the owner of daemon lifecycle, Invocation construction, prepare/sign/submit, stream, bidi, health, and typed errors.

Compatibility remains a profile client over governed daemon abilities. It exposes `CompatibilityListModelsRequest`, `CompatibilityChatCompletionRequest`, and `CompatibilityStreamChatCompletionRequest`; shorter legacy aliases are not part of the canonical SDK model.

Surface and Events profile request aliases that merely preserve old spelling are removed from the public Python root. Internal module implementation remains profile-owned.

The C ABI bridge is a private implementation detail for Python and a private adapter in Go. This iteration does not alter ABI symbols.
