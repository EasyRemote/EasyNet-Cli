# Invariants

1. `<agent>.discover` remains owner-namespaced and Device-hosted.
2. Runtime-backed discover tests declare their Device authority root explicitly.
3. Metadata-only discover tests declare the same Device authority root
   explicitly.
4. Provider delegation tests keep descriptor-bound targets and explicit
   subjects.
5. No production constructor behavior changes.
6. No fallback identity, compatibility route, or synthetic signer is introduced.
