# Intent

Close the remaining ambiguity around the legacy `invocation_history`
all-zero subject placeholder.

The placeholder may remain only as a negative test vector proving canonical
ingress rejection. It must never appear in production code, positive fixtures,
or product e2e scripts as a usable history subject.
