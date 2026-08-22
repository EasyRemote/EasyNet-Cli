# Intent

Fix the RemoteApp lifecycle E2E harnesses so public lifecycle calls use committed
Ability URAs instead of local short ability names.

The product goal remains incomplete. This change only removes a harness/runtime
boundary defect that prevented live timeout, cancel, and resume evidence from
being collected through the same public invocation surface that product callers
must use.

