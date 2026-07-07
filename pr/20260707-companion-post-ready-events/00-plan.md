# Companion Post-Ready Events Plan

## Goal

Complete the desktop companion Phase 5 requirement that companion startup after
daemon Ready is non-fatal, operator-visible, and reflected through the shared
runtime status model.

## Scope

- Keep daemon Ready ordering unchanged.
- Convert post-Ready companion startup failures from unstructured warning
  strings into typed reconciliation failures.
- Emit standard operator events at the CLI lifecycle boundary.
- Persist last-action failure memory in the companion state store.
- Project stored companion action failures through `DesktopCompanionStatus`.

## Non-Goals

- No product-specific SDK surface.
- No remote companion control.
- No new event bus.
- No alternate runtime lifecycle model.
