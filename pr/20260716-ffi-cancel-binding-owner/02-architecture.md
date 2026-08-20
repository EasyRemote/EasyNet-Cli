# Architecture

`ClientSessionBinding` is the canonical owner key for FFI invocation resources.
It carries both the public handle value and its incarnation, so registry cleanup
can distinguish a closed handle from a reused numeric slot.

`easynet_shutdown` already obtains the binding through `begin_closing` and then
calls `cancel_invocations_for_binding`. Keeping a second
`cancel_invocations_for_handle` helper duplicates ownership lookup and is only
used by a unit test. The test should exercise the binding-owned cleanup
directly.
