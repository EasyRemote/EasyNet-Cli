# Decisions Log

- Decision: implement one validator object instead of adding scattered `if` statements to `InvocationBuilder`.
- Decision: use canonical Java SDK authority error codes (`AUTHORITY_DENIED`, `AUTHORITY_SUBJECT_MISMATCH`) for tuple-binding failures rather than generic validation errors.
- Decision: update the Java delegation fixture so attached authority metadata is actually bound to the invocation subject and ability scope.
