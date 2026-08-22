# Intent — RemoteApp Route Visibility Gate

## Problem

The product-flow gate required daemon route projection and frontend ICE server
consumption, but did not require the browser UI to show the selected route
class. That leaves NAT/relay failures opaque.

## Change

- Require the frontend UI to render daemon route state in session details.
- Require UI coverage for `route host_only · no NAT/relay`.
- Update product readiness evidence without claiming network product
  completion.
