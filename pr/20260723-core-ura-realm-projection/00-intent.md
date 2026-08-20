# Intent

Move daemon-local realm extraction helpers onto the canonical URA facade.

The root defect is duplicated URA grammar projection in keyring and admission
modules. Even when each copy delegates to `parse_ura`, duplicating role-specific
realm helpers lets product subsystems grow independent URI/URA interpretations.
