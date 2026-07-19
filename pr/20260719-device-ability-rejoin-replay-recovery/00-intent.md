# Intent

Fix daemon boot after device rejoin when `device-abilities.json` still contains
durable installs owned by the previous device authority.

The daemon must not register old device-owned abilities under the new paired
device. It must also not fail with an opaque `errored=N` message that gives the
operator no recoverable state or offending authority root.

