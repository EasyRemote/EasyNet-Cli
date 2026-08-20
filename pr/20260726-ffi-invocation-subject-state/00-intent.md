Intent: close the daemon Ready compatibility window that allows a Device/Both runtime
to advertise attachable control discovery before paired User caller-signer custody is
part of the Ready contract.

Observed product symptom:
- remote canonical invocation can reach descriptor resolution with caller User URA,
  then fail because the local key service has no signer for that advertised User.

Architecture target:
- Ready must mean the local runtime has proven every identity/signing precondition
  required by canonical descriptor-bound invocation.
- Device/Both Ready requires paired User runtime signer custody.
- Hub Ready does not invent a paired User signer requirement.
