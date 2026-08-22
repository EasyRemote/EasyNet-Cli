# Intent — RemoteApp Capability Gate Hardening

The previous runtime capability fix separated production target subjects from
display-only diagnostic subjects. This batch hardens the existing lifecycle
input boundary gate so future edits cannot regress to raw descriptor-based
capability projection.

Scope is limited to boundary scripts, mutation tests, and product-closure
evidence. It does not add a frontend implementation directory or claim live
product completion.
