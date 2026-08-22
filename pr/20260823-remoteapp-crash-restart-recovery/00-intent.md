# Intent

Move RemoteApp crash/restart recovery from a verifier-only contract toward real
product behavior.

The current live probe shows that an active RemoteApp session is lost after an
unclean daemon restart: `remote_desktop.show_session` returns
`session_not_found` for the original `session_id`. This means the product cannot
claim crash/restart recovery, even though the evidence verifier exists.

This work must not make the verifier green by weakening its requirements.
Recovery must be implemented through daemon/plugin-owned durable session state
and public RemoteApp abilities.

