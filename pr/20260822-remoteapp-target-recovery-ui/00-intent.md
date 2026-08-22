# Intent — RemoteApp Target Recovery UI Gate

## Problem

RemoteApp product readiness depends on application/window capture and tracking
being understandable when they fail. The daemon already projects target
diagnostics, but the CLI product-flow gate did not require the frontend to show
target recovery state.

## Change

- Gate that the frontend RemoteApp session-details UI renders daemon-derived
  target status/reason/action.
- Gate a component test for `target lost · target_not_found · refresh_targets`.
- Record the evidence in the readiness matrix and product audit while keeping
  RemoteApp product status incomplete.

## Non-claim

This is not cross-platform capture completion and not multi-window churn E2E.
It is a product-observability closure for real application/window target loss.
