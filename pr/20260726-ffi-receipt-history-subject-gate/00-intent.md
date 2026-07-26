Goal
====

Close the FFI descriptor-resolution bypass that lets products resolve
`invocation.history.*` descriptors with a target Device subject.

Non-goals
=========

- Do not make the FFI layer synthesize receipt-history authority metadata.
- Do not add a compatibility fallback from Device subject to runtime-state read
  subject.
- Do not change receipt-history public ability names.

Acceptance criteria
===================

- Explicit `provider: "receipt_history"` descriptor requests require
  `subject_ura`.
- The required subject must be the canonical user-owned
  `resource/user.<id>/runtime-state/read` subject.
- Device subjects, retired session subjects, and all-zero user placeholders are
  rejected before descriptor materialization.
- Existing SDK/provider receipt-history guards remain unchanged.
