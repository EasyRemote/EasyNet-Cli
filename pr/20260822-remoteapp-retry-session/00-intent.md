# Intent — RemoteApp Retry Session Gate

## Problem

The frontend product-flow gate proved recovery-state visibility, but not that
`retry_session` guidance was executable. It also did not guard against terminal
session views blocking new `create_session` calls.

## Change

- Gate the frontend `Retry session` CTA.
- Gate component coverage proving retry calls `rdEnd` before `rdCreate`.
- Gate store coverage proving terminal sessions no longer block create.
- Update product readiness evidence without claiming full recovery completion.
