# Decisions Log

## 2026-07-16

- Chose to remove the retained schema payload instead of allowing dead code
  because the manifest/descriptor path already owns schemas.
- Kept constructor validation to preserve the fail-closed visibility rule for
  malformed manifests.
- Did not edit `agents/chat.rs` because current production prompt consumers
  already use only name and description and that file has unrelated dirty
  worktree changes.
