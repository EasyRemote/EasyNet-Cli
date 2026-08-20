# Decisions Log

1. Canonical authority metadata keys are now `x-runtime-delegation` and `x-runtime-session-authority`.
2. The cutover does not accept old `x-easynet-*` keys as aliases; accepting both would preserve a second canonical authority metadata contract.
3. Hosted-agent delegation metadata remains EasyNet provider policy and is intentionally out of scope for this canonical SDK/admission slice.
4. Cross-language SDKs and daemon admission use the same key literals while retaining each language's idiomatic constant naming.
5. The existing SDK conformance manifest did not require regeneration; the aggregate v2 gate passed after the source and fixture update.
