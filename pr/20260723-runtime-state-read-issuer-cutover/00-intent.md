# Intent

Cut daemon-local runtime-state read projections away from the generic
`invoke_local_ability` helper.

The generic helper still submits a daemon-self/local-daemon subject tuple. That
is acceptable only for product actions that intentionally run as daemon-local
system roots. It is not acceptable for read projections such as
`meta.list_abilities`, `meta.list_resources`, `observe.health`, and
`invocation.history.*`, because those reads must bind to the paired user's
runtime-state resource subject before admission.

This slice migrates remaining product read projections to
`LocalRuntimeStateReadIssuer` and extends the boundary gate so new runtime-state
reads cannot re-enter through the generic local invoke shortcut.
