# Voice Provider Boundary Gate

## Objective

Close the remaining A86/A89 regression surface with an executable architecture rule. Voice call signaling is Hub-owned realm state; live handlers may be registered only when a production-qualified realm-shared repository provider is assembled.

## Invariants

1. `VoiceCallProviderAssembly` is the only value accepted by live voice route registration.
2. Provider assembly validates repository qualification before exposing the repository.
3. `VoiceCallRepository` implementations must expose explicit qualification facts.
4. The test in-memory repository stays behind `cfg(test)` and is never production qualification evidence.
5. `HubRealmVoiceCallRepository` is constructed only from `EASYNET_HUB_VOICE_SHARED_ROOT`.
6. The production repository rejects relative roots and does not consult daemon-local state directories.
7. Catalog build registers `voice.*` handlers only when Hub authority and a provider assembly are both present.
8. Capability-state evidence reports `ProviderBacked` only from provider assembly, not from static descriptors.

## Effect

This slice does not change public behavior. It turns the current voice provider boundary into CI evidence so future edits cannot accidentally reintroduce process-local voice state or publish unavailable voice handlers.
