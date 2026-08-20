# Intent

Remove the remaining compatibility-shaped `fallback_code` vocabulary from the
bidi dispatch terminal-failure helper.

The helper does not provide a legacy execution path. It selects the default
failure code owned by the caller's terminal state machine when the reason text
does not prove a more specific runtime/admission code. The API and comments
must describe that policy directly.
