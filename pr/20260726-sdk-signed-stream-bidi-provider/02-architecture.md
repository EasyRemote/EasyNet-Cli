# Architecture

Layering:
- RuntimeTransport owns raw carrier submission.
- RuntimeClient owns public Runtime Core carrier APIs.
- AuthorizedRuntimeSession composes prepare/authorize/sign and delegates the signed carrier to RuntimeProvider.
- Product layers consume AuthorizedRuntimeSession; they do not implement stream/bidi fallbacks.

Boundary change:
- Add signed stream/bidi entrypoints at RuntimeClient, mirroring SubmitSigned.
- Remove the provider-unavailable adapter branch for signed stream/bidi.
