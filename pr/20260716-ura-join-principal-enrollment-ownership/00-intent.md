# URA Join Principal Enrollment Ownership

## Intent

Repair the URA join credential persistence path so it derives the local user id
from the optional principal enrollment proof before that proof is handed to the
federation join request.

## Boundary

This is a CLI join-stage ownership fix. It does not change federation wire
shape, receipt semantics, daemon routing, or stored credential schema.
