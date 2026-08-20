# Intent

Add desktop companion control to the Go/Python SDK parity matrix so the shipped
SDK facade is tracked by the same four-state capability model as the rest of
Runtime Core.

The implementation already exposes companion DTO and lifecycle wrappers in Go
and Python. This slice makes that support auditable through shared conformance
metadata instead of leaving it outside the canonical SDK capability matrix.
