# Architecture

## Boundary

`src/core/identity/mod.rs` owns dependency-free identity value guards.

## Refactoring direction

The existing implementation repeated the all-zero UUID sentinel in FFI invocation parsing, auth session loading, daemon credentials, and authority metadata. That duplication makes it easy for one ingress to check exact IDs while another checks embedded URAs differently.

The converged model exposes two semantic helpers:

- exact principal id placeholder check;
- embedded principal placeholder check for URA/payload fields.

Callers keep their local error domains but consume the same core predicate.
